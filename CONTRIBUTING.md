# Contributing to LLMSim

Thank you for your interest in contributing to LLMSim! This document provides guidelines for contributing to the project.

## Getting Started

### Prerequisites

- Rust 1.83 or later
- Git

### Setting Up the Development Environment

1. Fork the repository on GitHub
2. Clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/llmsim.git
   cd llmsim
   ```
3. Add the upstream remote:
   ```bash
   git remote add upstream https://github.com/llmsim/llmsim.git
   ```
4. Build the project:
   ```bash
   cargo build
   ```
5. Run the tests:
   ```bash
   cargo test
   ```

## Development Workflow

### Before Making Changes

1. Create a new branch for your work:
   ```bash
   git checkout -b feature/your-feature-name
   ```
2. Keep your branch up to date with upstream:
   ```bash
   git fetch upstream
   git rebase upstream/main
   ```

### Making Changes

1. Write your code following the existing code style
2. Add tests for new functionality
3. Ensure all tests pass:
   ```bash
   cargo test
   ```
4. Run the linter and formatter:
   ```bash
   cargo fmt
   cargo clippy --all-targets -- -D warnings
   ```

### Commit Guidelines

- Write clear, concise commit messages
- Use the imperative mood ("Add feature" not "Added feature")
- Reference issues when applicable (e.g., "Fix #123")

Example commit messages:
- `Add echo response generator`
- `Fix streaming response timing issue`
- `Update latency profile for GPT-4o`

### Submitting a Pull Request

1. Push your branch to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```
2. Open a Pull Request against the `main` branch
3. Fill out the PR template with a clear description of your changes
4. Wait for CI checks to pass
5. Address any review feedback

## Code Style

### Rust Guidelines

- Follow standard Rust naming conventions
- Use `rustfmt` for formatting
- Ensure `clippy` passes without warnings
- Document public APIs with doc comments
- Prefer explicit types over inference when it aids readability

### Documentation

- Update README.md if you add new features
- Add inline documentation for complex logic
- Include examples in doc comments where helpful

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name
```

### Writing Tests

- Place unit tests in the same file as the code being tested
- Use descriptive test names that explain what is being tested
- Test both success and error cases

## Reporting Issues

When reporting issues, please include:

1. A clear description of the problem
2. Steps to reproduce the issue
3. Expected behavior
4. Actual behavior
5. Your environment (OS, Rust version, etc.)

## Feature Requests

Feature requests are welcome! Please:

1. Check if the feature has already been requested
2. Provide a clear use case
3. Explain how the feature would benefit users

## Code of Conduct

- Be respectful and inclusive
- Provide constructive feedback
- Focus on the code, not the person
- Help others learn and grow

## License

By contributing to LLMSim, you agree that your contributions will be licensed under the MIT License.

## Questions?

Feel free to open an issue for questions or reach out to the maintainers.

Thank you for contributing!
