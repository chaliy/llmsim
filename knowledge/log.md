# LLMSim Knowledge Update Log

## 2026-11-12

* **Added the Latency Model.** New [Latency Model](simulation/latency.md) spec: measured
  GPT-6 tier profiles (Luna, Sol) and Muse Spark, log-normal TTFT, prompt-size prefill.
* **Built-in scenarios.** [Scenarios](simulation/scenarios.md) are compiled in and on by
  default; six agent scenarios from everruns joined the starter library.

## 2026-11-10

* **Added Python and Node bindings.** New [Python and Node Bindings](foundations/language-bindings.md)
  spec: PyPI and npm packages named `llmsim` embed the server in-process. The repo became a
  Cargo workspace; [Release Process](project/release-process.md) now publishes PyPI and npm.

## 2026-10-10

* **Added TypeSafe System One.** New [TypeSafe System One API Specification](apis/typesafe-api.md)
  for `/typesafe/v1/systemone` and `/typesafe/v1/models` (Jev model).
* **Adopted OKF v0.2.** The former `specs/` folder moved into this bundle, grouped into
  domains, with OKF frontmatter on every concept. `scripts/check_okf.py` and the
  `Knowledge Bundle (OKF v0.2)` CI job keep it conformant. See
  [Knowledge Maintenance Contract](knowledge-contract.md).
