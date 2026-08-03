# Windie Benchmark Fixtures

This directory stores benchmark artifacts that can be compared after code
changes.

Benchmark reports contain named scenario measurements. Each scenario records
its architectural layer and fixture so a regression can be attributed without
mixing unrelated work.

The default local benchmark covers SQLite/storage, context construction,
provider request serialization, runtime preparation, durable sessions, message
mutations, branching, and fake MCP protocol plus registry execution. API and
lifecycle scenarios are opt-in so the default remains a deterministic
provider-free baseline.

Run the additional local API and lifecycle scenarios with:

```bash
windie bench --all
```

The named layers are `storage`, `context`, `serialization`, `runtime`,
`sessions`, `mutations`, `mcp`, `api`, and `lifecycle`.

Conversation benchmark reports use the same named scenario structure and
include these lower-level layers:

```text
storage
sessions
context
serialization
```

Human-readable reports print every scenario's fixture, median, and p95 so a
timing is not interpreted without its workload. Storage includes both
row-only and complete-message tree/path pairs, plus a branched-tree case that
compares 1,000 total messages with a 100-message selected path.

Use the layer, scenario, and fixture together to locate regressions before
optimizing. The same fixture metadata is persisted in JSON reports.

## Conversation Fixture

Generate a report for one linear conversation tree:

```bash
windie bench <conversation-id> --runs 100 --json > benches/100-messages-current.json
```

The conversation ID is resolved through the backend-owned current session head
when one exists. Databases without a session use the latest tree row only as a
legacy-fixture fallback.

Compare two scenario reports:

```bash
windie bench compare benches/baseline.json benches/100-messages-current.json
```

Negative percentage changes mean the current code is faster. Positive
percentage changes mean the current code is slower.

Conversation benchmarks use the actual selected session head and measure the
current persisted conversation. Generate one with:

```bash
windie bench "$conversation_id" --runs 100 --json > benches/conversation-current.json
```

Compare two conversation reports:

```bash
windie bench compare benches/conversation-baseline.json benches/conversation-current.json
```

Narrow a run to selected layers with flags such as:

```bash
windie bench --conversation --serialization --runtime --sessions --api
```

External Bifrost inference, real provider/network behavior, browser Inspector
flows, process lifecycle, installer downloads, and release packaging remain
separate integration measurements. They are not included in the deterministic
default baseline.
