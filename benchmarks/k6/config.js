// k6 load testing configuration for llmsim
// This file contains shared configuration and utilities

// Default target URL (can be overridden via K6_TARGET_URL env var)
export const TARGET_URL = __ENV.K6_TARGET_URL || 'http://127.0.0.1:8080';

// Test profiles with configurable durations and VU counts
export const PROFILES = {
    // Smoke test: Quick validation that the system works
    smoke: {
        vus: 1,
        duration: '10s',
        iterations: 5,
        thresholds: {
            http_req_failed: ['rate<0.01'],      // <1% errors
            http_req_duration: ['p(95)<2000'],    // 95% under 2s
        },
    },

    // Load test: Normal expected load
    load: {
        stages: [
            { duration: '30s', target: 10 },   // Ramp up to 10 VUs
            { duration: '1m', target: 10 },    // Stay at 10 VUs
            { duration: '30s', target: 50 },   // Ramp up to 50 VUs
            { duration: '2m', target: 50 },    // Stay at 50 VUs
            { duration: '30s', target: 0 },    // Ramp down
        ],
        thresholds: {
            http_req_failed: ['rate<0.05'],      // <5% errors
            http_req_duration: ['p(95)<3000'],   // 95% under 3s
            http_req_duration: ['p(99)<5000'],   // 99% under 5s
        },
    },

    // Stress test: Push beyond normal capacity
    stress: {
        stages: [
            { duration: '30s', target: 50 },    // Ramp to 50 VUs
            { duration: '1m', target: 100 },    // Ramp to 100 VUs
            { duration: '1m', target: 200 },    // Ramp to 200 VUs
            { duration: '2m', target: 200 },    // Stay at 200 VUs
            { duration: '30s', target: 500 },   // Spike to 500 VUs
            { duration: '1m', target: 500 },    // Maintain spike
            { duration: '1m', target: 0 },      // Ramp down
        ],
        thresholds: {
            http_req_failed: ['rate<0.15'],     // <15% errors (stress allows more)
            http_req_duration: ['p(95)<10000'], // 95% under 10s
        },
    },

    // Spike test: Sudden burst of traffic
    spike: {
        stages: [
            { duration: '10s', target: 10 },    // Warm up
            { duration: '5s', target: 500 },    // Spike!
            { duration: '30s', target: 500 },   // Maintain spike
            { duration: '10s', target: 10 },    // Recover
            { duration: '30s', target: 10 },    // Continue at normal
            { duration: '10s', target: 0 },     // Ramp down
        ],
        thresholds: {
            http_req_failed: ['rate<0.20'],     // <20% errors during spike
        },
    },

    // Soak test: Sustained load over extended period (for memory leaks, etc.)
    soak: {
        stages: [
            { duration: '1m', target: 50 },     // Ramp up
            { duration: '30m', target: 50 },    // Sustained load
            { duration: '1m', target: 0 },      // Ramp down
        ],
        thresholds: {
            http_req_failed: ['rate<0.01'],     // Very low error rate
            http_req_duration: ['p(95)<3000'],  // Consistent latency
        },
    },

    // Quick smoke: Ultra-fast validation (2-3 iterations)
    'quick-smoke': {
        vus: 1,
        iterations: 3,
        thresholds: {
            http_req_failed: ['rate<0.01'],
        },
    },
};

// Available models for testing
export const MODELS = [
    'gpt-5',
    'gpt-5-mini',
    'gpt-4o',
    'gpt-4o-mini',
    'claude-opus-4',
    'claude-sonnet-4',
    'o3-mini',
];

// Sample prompts of varying complexity
export const PROMPTS = {
    short: [
        'Hello!',
        'Hi there!',
        'What is 2+2?',
    ],
    medium: [
        'Explain the concept of recursion in programming.',
        'What are the benefits of using async/await in JavaScript?',
        'Describe the differences between REST and GraphQL APIs.',
    ],
    long: [
        'Write a detailed explanation of how neural networks work, including the concepts of forward propagation, backpropagation, and gradient descent. Include practical examples.',
        'Explain the complete lifecycle of a web request from when a user types a URL in the browser to when the page is fully rendered, including DNS resolution, TCP handshake, TLS, HTTP, and rendering.',
    ],
};

// Get a random element from an array
export function randomChoice(arr) {
    return arr[Math.floor(Math.random() * arr.length)];
}

// Get a random model
export function randomModel() {
    return randomChoice(MODELS);
}

// Get a random prompt of specified type
export function randomPrompt(type = 'medium') {
    return randomChoice(PROMPTS[type] || PROMPTS.medium);
}

// Build a chat completion request payload
export function buildChatRequest(options = {}) {
    const {
        model = randomModel(),
        prompt = randomPrompt(),
        stream = false,
        maxTokens = 100,
    } = options;

    return JSON.stringify({
        model: model,
        messages: [
            { role: 'user', content: prompt }
        ],
        stream: stream,
        max_tokens: maxTokens,
    });
}

// Common HTTP request parameters
export const HTTP_PARAMS = {
    headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer test-api-key',
    },
    timeout: '60s',
};
