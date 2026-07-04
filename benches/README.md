# Windie Benchmark Fixtures

This directory stores benchmark artifacts that can be compared after code
changes.

Conversation benchmark reports include both top-level timings and lower-level
breakdowns:

```text
active message lookup
active path row load
active path part/image load
tree row load
tree part/image load
context active path load
context system prompt load
context compaction load
context flatten
```

Use the lower-level metrics to locate regressions before optimizing. For
example, if `tree part/image load` changes but `tree row load` is stable, the
pressure is in ordered message parts or image bytes rather than tree traversal.

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

## 100 Message Stress Fixture

`100-messages-stress-baseline.json` is a local/free benchmark report for one
mixed conversation tree:

```text
conversation_id: 8017e9d3-c859-4d3e-95c8-c2c982647858
active path messages: 100
tree messages: 164
```

The fixture includes a conversation-level system prompt, inserted system/user/
assistant/tool messages, single text messages, image-only user messages,
single text plus image messages, repeated image parts, repeated text parts,
interleaved text/image parts, one short inactive branch, and five moderate
inactive branch chains from the main path.

Regenerate a comparable stress report:

```bash
windie bench 8017e9d3-c859-4d3e-95c8-c2c982647858 --runs 100 --json > benches/100-messages-stress-current.json
```

Compare the saved stress baseline and current stress report:

```bash
windie bench compare benches/100-messages-stress-baseline.json benches/100-messages-stress-current.json
```
