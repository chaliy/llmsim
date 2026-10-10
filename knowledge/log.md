# LLMSim Knowledge Update Log

## 2026-10-10

* **Added TypeSafe System One.** New [TypeSafe System One API Specification](apis/typesafe-api.md)
  for `/typesafe/v1/systemone` and `/typesafe/v1/models` (Jev model).
* **Adopted OKF v0.2.** The former `specs/` folder moved into this bundle, grouped into
  domains, with OKF frontmatter on every concept. `scripts/check_okf.py` and the
  `Knowledge Bundle (OKF v0.2)` CI job keep it conformant. See
  [Knowledge Maintenance Contract](knowledge-contract.md).
