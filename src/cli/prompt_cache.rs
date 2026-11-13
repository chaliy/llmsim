// Simulated prompt caching.
//
// Providers bill (and serve faster) the part of a prompt they have seen
// recently. This keeps a TTL set of prompt-prefix hashes so repeated
// prefixes report cached tokens the way each API does:
//
// - OpenAI (automatic): prompts of at least 1024 tokens; the cached part is
//   the longest previously seen prefix, rounded down to 1024 + n*128 tokens.
//   Reported as `prompt_tokens_details.cached_tokens` (chat) and
//   `input_tokens_details.cached_tokens` (Responses).
// - Anthropic (explicit): only prefixes ending at a `cache_control`
//   breakpoint (or the last block, with top-level `cache_control`) are
//   cached. A seen breakpoint is a `cache_read_input_tokens` hit; the rest up
//   to the last breakpoint is `cache_creation_input_tokens`; `input_tokens`
//   is what follows.
//
// Decisions:
// - Prefix boundaries are message (and tool/system block) boundaries, not
//   tokens: real caches hit on the longest common prefix, and agent loops
//   (the main user) grow conversations by appending messages.
// - Cached tokens skip prefill latency, so a cache hit also starts sooner.
// - Entries live 5 minutes (refreshed on hit), like the providers' default.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Minimum prompt length that can be cached.
pub const MIN_CACHEABLE_TOKENS: usize = 1024;
/// OpenAI caches in 128-token increments beyond the minimum.
pub const OPENAI_CACHE_INCREMENT: usize = 128;
/// How long an unused prefix stays cached.
pub const CACHE_TTL: Duration = Duration::from_secs(300);
/// Entries kept before expired ones are swept.
const SWEEP_AT: usize = 100_000;

/// One prefix boundary: hash of everything up to it and its token count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefix {
    pub hash: u64,
    pub tokens: usize,
}

/// Builds cumulative prefix hashes over prompt pieces.
#[derive(Debug, Default)]
pub struct PrefixBuilder {
    hash: u64,
    tokens: usize,
    pub prefixes: Vec<Prefix>,
}

impl PrefixBuilder {
    pub fn new(namespace: &str) -> Self {
        let mut b = Self {
            hash: 0xcbf2_9ce4_8422_2325,
            ..Self::default()
        };
        b.mix(namespace.as_bytes());
        b
    }

    fn mix(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(0x0100_0000_01b3);
        }
        // Separator so ("ab","c") and ("a","bc") differ.
        self.hash ^= 0xff;
        self.hash = self.hash.wrapping_mul(0x0100_0000_01b3);
    }

    /// Append one piece (`tokens` long) and record the boundary after it.
    pub fn push(&mut self, piece: &str, tokens: usize) -> Prefix {
        self.mix(piece.as_bytes());
        self.tokens += tokens;
        let p = Prefix {
            hash: self.hash,
            tokens: self.tokens,
        };
        self.prefixes.push(p);
        p
    }
}

/// TTL set of seen prefixes.
#[derive(Debug, Default)]
pub struct PromptCache {
    entries: Mutex<HashMap<u64, Instant>>,
}

impl PromptCache {
    /// Longest prefix in `prefixes` seen within the TTL (its token count, 0
    /// if none), then remember every prefix in `store`.
    pub fn lookup_and_store(&self, prefixes: &[Prefix], store: &[Prefix], now: Instant) -> usize {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() > SWEEP_AT {
            entries.retain(|_, t| now.saturating_duration_since(*t) < CACHE_TTL);
        }
        let hit = prefixes
            .iter()
            .rev()
            .find(|p| {
                entries
                    .get(&p.hash)
                    .is_some_and(|t| now.saturating_duration_since(*t) < CACHE_TTL)
            })
            .map_or(0, |p| p.tokens);
        for p in store {
            entries.insert(p.hash, now);
        }
        hit
    }

    /// OpenAI automatic caching: cached tokens for a prompt of `total`
    /// tokens with message-boundary `prefixes`.
    pub fn openai(&self, prefixes: &[Prefix], total: usize, now: Instant) -> usize {
        if total < MIN_CACHEABLE_TOKENS {
            return 0;
        }
        let cacheable: Vec<Prefix> = prefixes
            .iter()
            .copied()
            .filter(|p| p.tokens >= MIN_CACHEABLE_TOKENS)
            .collect();
        let hit = self
            .lookup_and_store(&cacheable, &cacheable, now)
            .min(total);
        if hit < MIN_CACHEABLE_TOKENS {
            return 0;
        }
        MIN_CACHEABLE_TOKENS
            + (hit - MIN_CACHEABLE_TOKENS) / OPENAI_CACHE_INCREMENT * OPENAI_CACHE_INCREMENT
    }

    /// Anthropic explicit caching over `breakpoints` (prefixes that end at a
    /// `cache_control` block). Returns (cache_read, cache_creation) tokens.
    pub fn anthropic(&self, breakpoints: &[Prefix], now: Instant) -> (usize, usize) {
        let cacheable: Vec<Prefix> = breakpoints
            .iter()
            .copied()
            .filter(|p| p.tokens >= MIN_CACHEABLE_TOKENS)
            .collect();
        let Some(last) = cacheable.last() else {
            return (0, 0);
        };
        let read = self.lookup_and_store(&cacheable, &cacheable, now);
        (read, last.tokens.saturating_sub(read))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefixes(pieces: &[(&str, usize)]) -> Vec<Prefix> {
        let mut b = PrefixBuilder::new("m");
        for (p, t) in pieces {
            b.push(p, *t);
        }
        b.prefixes
    }

    #[test]
    fn openai_hits_the_longest_seen_prefix_in_128_steps() {
        let cache = PromptCache::default();
        let now = Instant::now();
        let first = prefixes(&[("system", 1500), ("user 1", 100)]);
        assert_eq!(cache.openai(&first, 1600, now), 0);
        // Same conversation plus one message: the 1600-token prefix hits.
        let second = prefixes(&[("system", 1500), ("user 1", 100), ("assistant", 50)]);
        assert_eq!(cache.openai(&second, 1650, now), 1024 + 4 * 128);
        // Short prompts never cache.
        let short = prefixes(&[("hi", 10)]);
        assert_eq!(cache.openai(&short, 10, now), 0);
        assert_eq!(cache.openai(&short, 10, now), 0);
    }

    #[test]
    fn entries_expire() {
        let cache = PromptCache::default();
        let now = Instant::now();
        let p = prefixes(&[("system", 2000)]);
        cache.openai(&p, 2000, now);
        assert_eq!(
            cache.openai(&p, 2000, now + CACHE_TTL + Duration::from_secs(1)),
            0
        );
    }

    #[test]
    fn anthropic_reads_then_creates() {
        let cache = PromptCache::default();
        let now = Instant::now();
        let bp = prefixes(&[("system", 3000)]);
        assert_eq!(cache.anthropic(&bp, now), (0, 3000));
        assert_eq!(cache.anthropic(&bp, now), (3000, 0));
        // A later breakpoint extends the cached prefix.
        let mut b = PrefixBuilder::new("m");
        let first = b.push("system", 3000);
        let second = b.push("history", 1000);
        assert_eq!(cache.anthropic(&[first, second], now), (3000, 1000));
        // Under the minimum nothing is cached.
        assert_eq!(cache.anthropic(&prefixes(&[("tiny", 10)]), now), (0, 0));
    }
}
