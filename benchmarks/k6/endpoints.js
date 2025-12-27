// k6 load test for all llmsim endpoints
// Tests health, models, stats, and chat completions
//
// Usage:
//   k6 run benchmarks/k6/endpoints.js
//   k6 run --env PROFILE=smoke benchmarks/k6/endpoints.js

import http from 'k6/http';
import { check, sleep, group } from 'k6';
import { Counter, Rate } from 'k6/metrics';
import { TARGET_URL, PROFILES, HTTP_PARAMS, buildChatRequest } from './config.js';

// Custom metrics
const endpointErrors = new Counter('endpoint_errors');
const healthChecks = new Counter('health_checks');
const modelListCalls = new Counter('model_list_calls');
const statsChecks = new Counter('stats_checks');

// Get profile from environment or default to 'smoke'
const profileName = __ENV.PROFILE || 'smoke';
const profile = PROFILES[profileName];

if (!profile) {
    throw new Error(`Unknown profile: ${profileName}`);
}

export const options = {
    scenarios: {
        endpoints: profile.stages
            ? { executor: 'ramping-vus', stages: profile.stages, gracefulRampDown: '30s' }
            : profile.iterations
                ? { executor: 'per-vu-iterations', vus: profile.vus, iterations: profile.iterations }
                : { executor: 'constant-vus', vus: profile.vus, duration: profile.duration },
    },
    thresholds: {
        http_req_failed: ['rate<0.05'],
        http_req_duration: ['p(95)<5000'],
        ...profile.thresholds,
    },
};

export default function () {
    // Weighted endpoint selection
    const rand = Math.random();

    if (rand < 0.05) {
        // 5% health checks
        group('health', () => testHealth());
    } else if (rand < 0.10) {
        // 5% model list
        group('models', () => testModels());
    } else if (rand < 0.15) {
        // 5% stats
        group('stats', () => testStats());
    } else if (rand < 0.20) {
        // 5% model detail
        group('model_detail', () => testModelDetail());
    } else {
        // 80% chat completions
        group('chat', () => testChatCompletion());
    }

    sleep(0.05 + Math.random() * 0.1);
}

function testHealth() {
    const response = http.get(`${TARGET_URL}/health`);
    healthChecks.add(1);

    const success = check(response, {
        'health status 200': (r) => r.status === 200,
        'health has status field': (r) => {
            try {
                return JSON.parse(r.body).status === 'ok';
            } catch (e) {
                return false;
            }
        },
    });

    if (!success) endpointErrors.add(1);
}

function testModels() {
    const response = http.get(`${TARGET_URL}/v1/models`);
    modelListCalls.add(1);

    const success = check(response, {
        'models status 200': (r) => r.status === 200,
        'models has data array': (r) => {
            try {
                const body = JSON.parse(r.body);
                return body.data && Array.isArray(body.data);
            } catch (e) {
                return false;
            }
        },
        'models object is list': (r) => {
            try {
                return JSON.parse(r.body).object === 'list';
            } catch (e) {
                return false;
            }
        },
    });

    if (!success) endpointErrors.add(1);
}

function testModelDetail() {
    const models = ['gpt-5', 'gpt-4o', 'claude-opus-4'];
    const model = models[Math.floor(Math.random() * models.length)];

    const response = http.get(`${TARGET_URL}/v1/models/${model}`);

    const success = check(response, {
        'model detail status 200': (r) => r.status === 200,
        'model has id': (r) => {
            try {
                return JSON.parse(r.body).id !== undefined;
            } catch (e) {
                return false;
            }
        },
    });

    if (!success) endpointErrors.add(1);
}

function testStats() {
    const response = http.get(`${TARGET_URL}/llmsim/stats`);
    statsChecks.add(1);

    const success = check(response, {
        'stats status 200': (r) => r.status === 200,
        'stats has total_requests': (r) => {
            try {
                return JSON.parse(r.body).total_requests !== undefined;
            } catch (e) {
                return false;
            }
        },
        'stats has uptime': (r) => {
            try {
                return JSON.parse(r.body).uptime_secs !== undefined;
            } catch (e) {
                return false;
            }
        },
    });

    if (!success) endpointErrors.add(1);
}

function testChatCompletion() {
    const useStreaming = Math.random() > 0.5;
    const payload = buildChatRequest({
        stream: useStreaming,
        maxTokens: 50,
    });

    const response = http.post(
        `${TARGET_URL}/v1/chat/completions`,
        payload,
        {
            ...HTTP_PARAMS,
            responseType: useStreaming ? 'text' : 'none',
        }
    );

    const success = check(response, {
        'chat status 200': (r) => r.status === 200,
        'chat has response': (r) => {
            if (useStreaming) {
                return r.body && r.body.includes('data:');
            } else {
                try {
                    return JSON.parse(r.body).choices !== undefined;
                } catch (e) {
                    return false;
                }
            }
        },
    });

    if (!success) endpointErrors.add(1);
}

export function setup() {
    console.log(`\n=== llmsim Endpoint Test ===`);
    console.log(`Profile: ${profileName}`);
    console.log(`Target: ${TARGET_URL}`);
    console.log(`============================\n`);

    const healthCheck = http.get(`${TARGET_URL}/health`);
    if (healthCheck.status !== 200) {
        throw new Error(`Server at ${TARGET_URL} is not reachable`);
    }

    return { startTime: Date.now() };
}

export function teardown(data) {
    const duration = (Date.now() - data.startTime) / 1000;
    console.log(`\n=== Test Complete (${duration.toFixed(1)}s) ===\n`);
}
