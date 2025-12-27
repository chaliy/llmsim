// k6 high-concurrency load test for llmsim
// Specifically designed to test llmsim's ability to handle many concurrent connections
//
// Usage:
//   k6 run benchmarks/k6/high-concurrency.js
//   k6 run --env MAX_VUS=1000 benchmarks/k6/high-concurrency.js
//   k6 run --env RAMP_DURATION=30s --env HOLD_DURATION=2m benchmarks/k6/high-concurrency.js

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter, Trend, Gauge } from 'k6/metrics';
import { TARGET_URL, HTTP_PARAMS, buildChatRequest, randomModel } from './config.js';

// Custom metrics for high-concurrency testing
const concurrentRequests = new Gauge('concurrent_requests');
const requestsPerVU = new Counter('requests_per_vu');
const connectionErrors = new Counter('connection_errors');
const timeoutErrors = new Counter('timeout_errors');
const responseLatency = new Trend('response_latency', true);

// Configuration from environment
const MAX_VUS = parseInt(__ENV.MAX_VUS || '500');
const RAMP_DURATION = __ENV.RAMP_DURATION || '1m';
const HOLD_DURATION = __ENV.HOLD_DURATION || '2m';
const RAMP_DOWN = __ENV.RAMP_DOWN || '30s';

export const options = {
    scenarios: {
        high_concurrency: {
            executor: 'ramping-vus',
            startVUs: 1,
            stages: [
                { duration: RAMP_DURATION, target: MAX_VUS },  // Ramp up
                { duration: HOLD_DURATION, target: MAX_VUS }, // Hold at max
                { duration: RAMP_DOWN, target: 0 },           // Ramp down
            ],
            gracefulRampDown: '30s',
        },
    },
    thresholds: {
        http_req_failed: ['rate<0.10'],         // Allow up to 10% failures under extreme load
        http_req_duration: ['p(95)<15000'],     // 95th percentile under 15s
        connection_errors: ['count<100'],        // Max 100 connection errors
        timeout_errors: ['count<50'],            // Max 50 timeouts
    },
    // Connection settings for high concurrency
    batch: 20,
    batchPerHost: 20,
    dns: {
        ttl: '1m',
        select: 'roundRobin',
    },
};

// Track active requests per VU
let activeRequests = 0;

export default function () {
    activeRequests++;
    concurrentRequests.add(activeRequests);

    const startTime = Date.now();

    // Mix of request types weighted towards quick non-streaming for max throughput
    const requestType = Math.random();

    let response;
    try {
        if (requestType < 0.7) {
            // 70% quick non-streaming requests
            response = quickNonStreaming();
        } else if (requestType < 0.9) {
            // 20% streaming requests
            response = streamingRequest();
        } else {
            // 10% health/stats checks (lightweight)
            response = lightweightCheck();
        }

        const latency = Date.now() - startTime;
        responseLatency.add(latency);
        requestsPerVU.add(1);

    } catch (e) {
        if (e.message && e.message.includes('timeout')) {
            timeoutErrors.add(1);
        } else {
            connectionErrors.add(1);
        }
    } finally {
        activeRequests--;
        concurrentRequests.add(activeRequests);
    }

    // Minimal sleep to maximize request rate
    sleep(0.01 + Math.random() * 0.05);
}

function quickNonStreaming() {
    const payload = buildChatRequest({
        stream: false,
        maxTokens: 20, // Small response for speed
        prompt: 'Hi',
    });

    const response = http.post(
        `${TARGET_URL}/v1/chat/completions`,
        payload,
        {
            ...HTTP_PARAMS,
            timeout: '30s',
        }
    );

    check(response, {
        'quick response ok': (r) => r.status === 200,
    });

    return response;
}

function streamingRequest() {
    const payload = buildChatRequest({
        stream: true,
        maxTokens: 50,
    });

    const response = http.post(
        `${TARGET_URL}/v1/chat/completions`,
        payload,
        {
            ...HTTP_PARAMS,
            responseType: 'text',
            timeout: '60s',
        }
    );

    check(response, {
        'streaming ok': (r) => r.status === 200 && r.body.includes('[DONE]'),
    });

    return response;
}

function lightweightCheck() {
    const endpoint = Math.random() > 0.5 ? '/health' : '/llmsim/stats';
    const response = http.get(`${TARGET_URL}${endpoint}`, {
        timeout: '10s',
    });

    check(response, {
        'lightweight ok': (r) => r.status === 200,
    });

    return response;
}

export function setup() {
    console.log(`\n=== High-Concurrency Load Test ===`);
    console.log(`Target: ${TARGET_URL}`);
    console.log(`Max VUs: ${MAX_VUS}`);
    console.log(`Ramp: ${RAMP_DURATION} -> Hold: ${HOLD_DURATION} -> Down: ${RAMP_DOWN}`);
    console.log(`==================================\n`);

    // Warm-up and verify server
    const healthCheck = http.get(`${TARGET_URL}/health`);
    if (healthCheck.status !== 200) {
        throw new Error(`Server not reachable at ${TARGET_URL}`);
    }

    // Get initial stats
    const initialStats = http.get(`${TARGET_URL}/llmsim/stats`);
    let initialData = {};
    if (initialStats.status === 200) {
        try {
            initialData = JSON.parse(initialStats.body);
        } catch (e) {}
    }

    return {
        startTime: Date.now(),
        maxVUs: MAX_VUS,
        initialRequests: initialData.total_requests || 0,
    };
}

export function teardown(data) {
    const duration = (Date.now() - data.startTime) / 1000;

    // Get final stats
    const finalStats = http.get(`${TARGET_URL}/llmsim/stats`);
    let finalData = {};
    if (finalStats.status === 200) {
        try {
            finalData = JSON.parse(finalStats.body);
        } catch (e) {}
    }

    const totalRequests = (finalData.total_requests || 0) - data.initialRequests;
    const rps = totalRequests / duration;

    console.log(`\n=== High-Concurrency Test Results ===`);
    console.log(`Duration: ${duration.toFixed(1)}s`);
    console.log(`Max VUs: ${data.maxVUs}`);
    console.log(`Total Requests: ${totalRequests}`);
    console.log(`Average RPS: ${rps.toFixed(1)}`);
    console.log(`Peak RPS: ${finalData.requests_per_second?.toFixed(1) || 'N/A'}`);
    console.log(`Avg Latency: ${finalData.avg_latency_ms?.toFixed(1) || 'N/A'}ms`);
    console.log(`Total Tokens: ${finalData.total_tokens || 0}`);
    console.log(`=====================================\n`);
}
