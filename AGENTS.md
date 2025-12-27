## Coding-agent guidance (repo root)

This repo is intended to be runnable locally and easy for coding agents to work in.

### Principles

- Keep decisions as comments on top of the file. Only important decisions that could not be inferred from code.
- Code should be easily testable, smoke testable, runnable in local dev env.
- Prefer small, incremental PR-sized changes with a runnable state at each step.
- Avoid adding dependencies with non-permissive licenses. If a dependency is non-permissive or unclear, stop and ask the repo owner.

### Top level requirements

....

### Specs

`specs/` folder contains feature specifications outlining requirements for specific features and components. New code should comply with these specifications or propose changes to them.

Available specs:
- `specs/architecture.md` - System architecture and module organization
- `specs/load-testing.md` - Load testing framework and benchmarks

Specification format: Abstract and Requirements sections.

### Skills

`.claude/skills/` contains development skills following the [Agent Skills Specification](https://agentskills.io/specification).

Available skills:
- `load-test/` - Run load and stress tests for llmsim using k6


### Public Documentation

`docs/` contains public-facing user documentation. This documentation is intended for end users and operators of the system, not for internal development reference.


When making changes that affect user-facing behavior or operations, update the relevant docs in this folder.

### Local dev expectations

....

### Cloud Agent environments

When running in cloud-hosted agent environments (e.g., Claude Code on the web), the following secrets are available:

- `OPENAI_API_KEY`: Available for LLM-related operations (OpenAI models)
- `ANTHROPIC_API_KEY`: Available for LLM-related operations (Claude models)
- `GITHUB_TOKEN`: Available for GitHub API operations (PRs, issues, repository access)

These secrets are pre-configured in the environment and do not require manual setup.

### Conventions

#### Code organization

....

#### Naming

....


### CI expectations

- CI is implemented using GitHub Actions.
....

### Pre-PR checklist

Before creating a pull request, ensure:

1. **Formatting**: Run ... to format all code
2. **Linting**: Run ... and fix all warnings
3. **Tests**: Run ... to ensure all tests pass
4. **Smoke tests**: Run smoke tests to verify the system works end-to-end
5. **Update specs**: If your changes affect system behavior, update the relevant specs in `specs/`
6. **Update docs**: If your changes affect usage or configuration, update public docs in `./docs` folder

CI will fail if formatting, linting, tests, or UI build fail. Always run these locally before pushing.

### Commit message conventions

Follow [Conventional Commits](https://www.conventionalcommits.org) for all commit messages:

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style (formatting, semicolons, etc.)
- `refactor`: Code refactoring without feature/fix
- `perf`: Performance improvements
- `test`: Adding or updating tests
- `chore`: Build process, dependencies, tooling
- `ci`: CI configuration changes

**Examples:**
```
feat(api): add agent versioning endpoint
fix(workflow): handle timeout in run execution
docs: update API documentation
refactor(db): simplify connection pooling
```

**Validation (optional):**
```bash
# Validate a commit message
echo "feat: add new feature" | npx commitlint

# Validate last commit
npx commitlint --from HEAD~1 --to HEAD
```

### PR (Pull Request) conventions

PR titles should follow Conventional Commits format. Use the PR template (`.github/pull_request_template.md`) for descriptions.

**PR Body Template:**

```markdown
## What
Clear description of the change.

## Why
Problem or motivation.

## How
High-level approach.

## Risk
- Low / Medium / High
- What can break

## Checklist
- [ ] Tests added or updated
- [ ] Backward compatibility considered
```

## Testing the system

...

