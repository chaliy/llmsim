// k6 load test for llmsim chat completions endpoint
// Tests both streaming and non-streaming chat completions
//
// Usage:
//   k6 run benchmarks/k6/chat-completions.js
//   k6 run --env PROFILE=smoke benchmarks/k6/chat-completions.js
//   k6 run --env PROFILE=load benchmarks/k6/chat-completions.js
//   k6 run --env PROFILE=stress benchmarks/k6/chat-completions.js

import http from 'k6/http';
import { check, sleep, group } from 'k6';
import { Counter, Rate, Trend } from 'k6/metrics';
import { TARGET_URL, PROFILES, HTTP_PARAMS, buildChatRequest, randomModel } from './config.js';

// Custom metrics
const chatCompletionErrors = new Counter('chat_completion_errors');
const streamingRequests = new Counter('streaming_requests');
const nonStreamingRequests = new Counter('non_streaming_requests');
const tokensThroughput = new Counter('tokens_processed');
const timeToFirstToken = new Trend('time_to_first_token', true);
const streamingDuration = new Trend('streaming_duration', true);

// Get profile from environment or default to 'load'
const profileName = __ENV.PROFILE || 'load';
const profile = PROFILES[profileName];

if (!profile) {
    console.error(`Unknown profile: ${profileName}. Available: ${Object.keys(PROFILES).join(', ')}`);
    throw new Error(`Unknown profile: ${profileName}`);
}

// Export k6 options based on selected profile
export const options = {
    scenarios: {
        chat_completions: profile.stages
            ? { executor: 'ramping-vus', stages: profile.stages, gracefulRampDown: '30s' }
            : profile.iterations
                ? { executor: 'per-vu-iterations', vus: profile.vus, iterations: profile.iterations }
                : { executor: 'constant-vus', vus: profile.vus, duration: profile.duration },
    },
    thresholds: profile.thresholds || {},
};

// Main test function
export default function () {
    // Randomly choose between streaming and non-streaming
    const useStreaming = Math.random() > 0.3; // 70% streaming, 30% non-streaming

    if (useStreaming) {
        group('streaming_chat_completion', () => {
            testStreamingChatCompletion();
        });
    } else {
        group('non_streaming_chat_completion', () => {
            testNonStreamingChatCompletion();
        });
    }

    // Small sleep between requests
    sleep(0.1 + Math.random() * 0.2);
}

function testNonStreamingChatCompletion() {
    const payload = buildChatRequest({
        stream: false,
        maxTokens: 50 + Math.floor(Math.random() * 100),
    });

    const response = http.post(
        `${TARGET_URL}/openai/v1/chat/completions`,
        payload,
        HTTP_PARAMS
    );

    nonStreamingRequests.add(1);

    const success = check(response, {
        'status is 200': (r) => r.status === 200,
        'has choices': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.choices && body.choices.length > 0;
            } catch (e) {
                return false;
            }
        },
        'has usage info': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.usage && body.usage.total_tokens > 0;
            } catch (e) {
                return false;
            }
        },
    });

    if (!success) {
        chatCompletionErrors.add(1);
    } else {
        try {
            const body = JSON.parse(response.body);
            if (body.usage) {
                tokensThroughput.add(body.usage.total_tokens);
            }
        } catch (e) {
            // Ignore parse errors
        }
    }
}

function testStreamingChatCompletion() {
    const payload = buildChatRequest({
        stream: true,
        maxTokens: 50 + Math.floor(Math.random() * 150),
    });

    const startTime = Date.now();
    let firstChunkTime = null;

    const response = http.post(
        `${TARGET_URL}/openai/v1/chat/completions`,
        payload,
        {
            ...HTTP_PARAMS,
            responseType: 'text', // Get full response as text for SSE
        }
    );

    streamingRequests.add(1);
    const endTime = Date.now();

    const success = check(response, {
        'status is 200': (r) => r.status === 200,
        'is SSE format': (r) => r.body && r.body.includes('data:'),
        'has completion': (r) => r.body && r.body.includes('[DONE]'),
        'has content chunks': (r) => r.body && r.body.includes('"delta"'),
    });

    if (!success) {
        chatCompletionErrors.add(1);
    } else {
        // Parse SSE response to extract metrics
        const body = response.body || '';
        const lines = body.split('\n').filter(l => l.startsWith('data:'));

        if (lines.length > 1) {
            // Estimate time to first token (first content chunk)
            // This is approximate since we don't have precise timing per chunk
            const estimatedTTFT = response.timings.waiting;
            timeToFirstToken.add(estimatedTTFT);
        }

        // Count approximate tokens from content chunks
        let tokenCount = 0;
        lines.forEach(line => {
            if (line.includes('"content"')) {
                tokenCount++;
            }
        });
        tokensThroughput.add(tokenCount);

        streamingDuration.add(endTime - startTime);
    }
}

// Setup function (runs once before test)
export function setup() {
    console.log(`\n=== llmsim Load Test ===`);
    console.log(`Profile: ${profileName}`);
    console.log(`Target: ${TARGET_URL}`);
    console.log(`========================\n`);

    // Verify server is reachable
    const healthCheck = http.get(`${TARGET_URL}/health`);
    const healthy = check(healthCheck, {
        'server is healthy': (r) => r.status === 200,
    });

    if (!healthy) {
        throw new Error(`Server at ${TARGET_URL} is not reachable`);
    }

    return {
        startTime: Date.now(),
        profile: profileName,
    };
}

// Teardown function (runs once after test)
export function teardown(data) {
    const duration = (Date.now() - data.startTime) / 1000;
    console.log(`\n=== Test Complete ===`);
    console.log(`Profile: ${data.profile}`);
    console.log(`Duration: ${duration.toFixed(1)}s`);
    console.log(`=====================\n`);

    // Fetch final stats from llmsim
    const stats = http.get(`${TARGET_URL}/llmsim/stats`);
    if (stats.status === 200) {
        try {
            const statsData = JSON.parse(stats.body);
            console.log(`Server Stats:`);
            console.log(`  Total Requests: ${statsData.total_requests}`);
            console.log(`  Total Tokens: ${statsData.total_tokens}`);
            console.log(`  Avg Latency: ${statsData.avg_latency_ms?.toFixed(1)}ms`);
            console.log(`  RPS: ${statsData.requests_per_second?.toFixed(1)}`);
        } catch (e) {
            // Ignore
        }
    }
}
