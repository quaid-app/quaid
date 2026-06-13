## 1. OpenSpec and root-cause coverage

- [x] 1.1 Record the playground warning fix as a scoped OpenSpec change before code changes land.
- [x] 1.2 Add regression coverage for the empty-chunk embedding path triggered by canonical conversation day-files.
- [x] 1.3 Add regression coverage through the worker path for the one-turn coffee-vs-tea preference case when the SLM wraps valid JSON in commentary.

## 2. Implementation

- [x] 2.1 Fix conversation chunking so canonical day-files do not emit blank chunks into embedding refresh.
- [x] 2.2 Keep the prompt guidance, but harden SLM response parsing so exactly one plain-commentary-wrapped JSON envelope still recovers while multi-object, schema-example, and non-envelope structural wrappers fail closed.

## 3. Validation

- [x] 3.1 Run the targeted extraction / prompt / embedding regression tests and confirm they pass.
