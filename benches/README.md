# Windie Benchmark Fixtures

This directory stores benchmark artifacts that can be compared after code
changes.

## 100 Messages

`100-messages-baseline.json` is a local/free benchmark report for one linear
conversation tree with 100 messages.

Regenerate a comparable current report:

```bash
windie bench <conversation-id> --runs 100 --json > benches/100-messages-current.json
```

Compare the saved baseline and current report:

```bash
windie bench compare benches/100-messages-baseline.json benches/100-messages-current.json
```

Negative percentage changes mean the current code is faster. Positive
percentage changes mean the current code is slower.
