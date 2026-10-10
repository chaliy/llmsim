// System One answer synthesis
//
// Decision: answers are a deterministic function of (state, question name,
// question), seeded by FNV-1a. The same request always gets the same answers,
// so tests asserting on thresholds and routing stay stable, while different
// questions and states still spread across the whole probability range.
// FNV-1a is used instead of `DefaultHasher` because std does not promise that
// hasher's output stays the same across Rust releases.
//
// Every answer satisfies the invariants real clients check (everruns'
// System One decoder rejects responses that break them): probabilities in
// 0..=1 summing to 1, a choice that names one of the options, a score inside
// 0..=levels-1, and a confidence in 0..=1.

use super::types::{Answer, Question, QuestionKind, SystemOneRequest};
use serde_json::Value;
use std::collections::BTreeMap;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(seed, |h, b| (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME))
}

/// SplitMix64: a tiny, well-mixed PRNG for turning one seed into several draws.
struct SplitMix(u64);

impl SplitMix {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn question_seed(state: &Value, name: &str, question: &Question) -> u64 {
    let mut h = fnv1a(FNV_OFFSET, state.to_string().as_bytes());
    h = fnv1a(h, name.as_bytes());
    h = fnv1a(h, question.kind.name().as_bytes());
    if let Some(instructions) = &question.instructions {
        h = fnv1a(h, instructions.to_string().as_bytes());
    }
    h
}

/// Softmax over logits drawn from `rng`, with a sharpness drawn per question so
/// some answers are confident and others split.
fn distribution(rng: &mut SplitMix, n: usize) -> Vec<f64> {
    let sharpness = 1.0 + rng.next_f64() * 4.0;
    let logits: Vec<f64> = (0..n).map(|_| rng.next_f64() * sharpness).collect();
    let max = logits.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = logits.iter().map(|l| (l - max).exp()).collect();
    let sum: f64 = exps.iter().sum();
    exps.into_iter().map(|e| e / sum).collect()
}

/// `1 - normalized entropy`: 1 for a one-hot distribution, 0 for uniform.
fn confidence(probabilities: &[f64]) -> f64 {
    if probabilities.len() < 2 {
        return 1.0;
    }
    let entropy: f64 = probabilities
        .iter()
        .filter(|p| **p > 0.0)
        .map(|p| -p * p.ln())
        .sum();
    (1.0 - entropy / (probabilities.len() as f64).ln()).clamp(0.0, 1.0)
}

/// Synthesize the answer to one question.
pub fn answer_question(state: &Value, name: &str, question: &Question) -> Answer {
    let mut rng = SplitMix(question_seed(state, name, question));
    match &question.kind {
        QuestionKind::Noul { .. } => {
            // Logistic over a logit in [-4, 4]: lands in roughly 0.02..0.98.
            let logit = rng.next_f64() * 8.0 - 4.0;
            Answer::Noul {
                noul: 1.0 / (1.0 + (-logit).exp()),
            }
        }
        QuestionKind::Choice { options } => {
            let probs = distribution(&mut rng, options.len());
            let best = probs
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or(0, |(i, _)| i);
            Answer::Choice {
                choice: options[best].0.clone(),
                confidence: confidence(&probs),
                probabilities: options
                    .iter()
                    .map(|(o, _)| o.clone())
                    .zip(probs.iter().copied())
                    .collect(),
            }
        }
        QuestionKind::Score { levels } => {
            let probs = distribution(&mut rng, levels.len());
            let score = probs
                .iter()
                .enumerate()
                .map(|(i, p)| i as f64 * p)
                .sum::<f64>()
                .clamp(0.0, (levels.len() - 1) as f64);
            Answer::Score {
                score,
                confidence: confidence(&probs),
                legend: levels
                    .iter()
                    .enumerate()
                    .map(|(i, l)| (i.to_string(), l.clone()))
                    .collect(),
                probabilities: probs
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (i.to_string(), *p))
                    .collect(),
            }
        }
    }
}

/// Synthesize answers to every question in a request.
pub fn answer_all(request: &SystemOneRequest) -> BTreeMap<String, Answer> {
    request
        .questions
        .iter()
        .map(|(name, q)| (name.clone(), answer_question(&request.state, name, q)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request(questions: Value) -> SystemOneRequest {
        SystemOneRequest::parse(&json!({
            "state": "I was charged twice. Please fix this ASAP.",
            "model": "jev-latest",
            "questions": questions,
        }))
        .unwrap()
    }

    fn sums_to_one(map: &BTreeMap<String, f64>) -> bool {
        map.values().all(|p| (0.0..=1.0).contains(p))
            && (map.values().sum::<f64>() - 1.0).abs() < 1e-9
    }

    #[test]
    fn answers_are_deterministic() {
        let req = request(json!({"q": {"type": "noul", "instructions": "Billing?"}}));
        assert_eq!(answer_all(&req), answer_all(&req));
    }

    #[test]
    fn different_questions_get_different_answers() {
        let req = request(json!({
            "a": {"type": "noul", "instructions": "Billing?"},
            "b": {"type": "noul", "instructions": "Shipping?"}
        }));
        let answers = answer_all(&req);
        assert_ne!(answers["a"], answers["b"]);
    }

    #[test]
    fn answers_satisfy_wire_invariants_across_many_seeds() {
        for i in 0..200 {
            let req = request(json!({
                "n": {"type": "noul", "instructions": format!("q{i}")},
                "c": {"type": "choice", "instructions": format!("q{i}"),
                      "criteria": {"a": null, "b": "B", "c": {"x": 1}}},
                "s": {"type": "score", "instructions": format!("q{i}"),
                      "criteria": ["lo", "mid", "hi", "max"]},
                "one": {"type": "score", "instructions": format!("q{i}"), "criteria": ["only"]}
            }));
            let answers = answer_all(&req);
            match &answers["n"] {
                Answer::Noul { noul } => assert!((0.0..=1.0).contains(noul)),
                other => panic!("{other:?}"),
            }
            match &answers["c"] {
                Answer::Choice {
                    choice,
                    confidence,
                    probabilities,
                } => {
                    assert!(sums_to_one(probabilities));
                    assert!(probabilities.contains_key(choice));
                    let best = probabilities.values().cloned().fold(0.0, f64::max);
                    assert_eq!(probabilities[choice], best);
                    assert!((0.0..=1.0).contains(confidence));
                }
                other => panic!("{other:?}"),
            }
            match &answers["s"] {
                Answer::Score {
                    score,
                    confidence,
                    legend,
                    probabilities,
                } => {
                    assert!(sums_to_one(probabilities));
                    assert!((0.0..=3.0).contains(score));
                    assert_eq!(legend.len(), 4);
                    assert_eq!(legend["0"], json!("lo"));
                    assert!((0.0..=1.0).contains(confidence));
                }
                other => panic!("{other:?}"),
            }
            match &answers["one"] {
                Answer::Score {
                    score, confidence, ..
                } => {
                    assert_eq!(*score, 0.0);
                    assert_eq!(*confidence, 1.0);
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn confidence_is_one_for_one_hot_and_zero_for_uniform() {
        assert_eq!(confidence(&[1.0, 0.0, 0.0]), 1.0);
        assert!(confidence(&[0.25; 4]).abs() < 1e-12);
    }
}
