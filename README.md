# ZylonDB

**Fast by nature. Simple by design.**

A lightweight, high-performance embedded document database for Node.js, powered by Rust.

> **Project status:** Early development. ZylonDB is not yet production-ready.

## Why ZylonDB?

Modern applications shouldn't need complicated infrastructure just to store and query data.

ZylonDB aims to combine the simplicity of document databases with the convenience of embedded storage and the performance potential of Rust.

### Design Principles

- **Zero Setup** — No separate database server or Docker container.
- **Simple Queries** — An intuitive TypeScript API for querying JSON documents.
- **Performance First** — Efficient indexing, native query execution, and optimized storage.
- **Persistent Storage** — Local data persistence with crash recovery.
- **Developer Friendly** — Designed for Node.js applications.
- **Open Source** — Built transparently with community contributions.

## Planned Developer Experience

The following API illustrates the intended design. It is not yet available as a published implementation.

```typescript
import { ZylonDB } from "zylondb";

const db = await ZylonDB.open("./data");

const users = db.collection("users");

await users.insert({
  name: "Alice",
  age: 28,
  city: "Chennai"
});

const results = await users
  .where("age")
  .gte(18)
  .sort("age", "desc")
  .limit(10)
  .get();

console.log(results);
```

## Architecture

ZylonDB will use two primary technologies:

- **Rust:** Storage engine, indexing, query execution, and persistence.
- **TypeScript:** Node.js SDK, developer-facing API, and tooling.

Native Node.js bindings will be implemented using `napi-rs`.

## Roadmap

### v0.1 — Core Storage
- [ ] Append-only storage engine
- [ ] Basic key-value operations
- [ ] Primary-key indexing
- [ ] Persistence and crash recovery
- [ ] Node.js native bindings

### v0.2 — Document Database
- [ ] JSON document collections
- [ ] CRUD operations
- [ ] Simple query builder
- [ ] Secondary indexes
- [ ] Sorting and pagination

### v0.3 — Reliability
- [ ] Compaction
- [ ] Database locking
- [ ] Crash and corruption testing
- [ ] Backup and restore
- [ ] File-format versioning

### v0.4 — Performance
- [ ] Query optimization
- [ ] Memory caching
- [ ] Batched operations
- [ ] Reproducible benchmarks

### v1.0 — Stable Release
- [ ] Stable public API
- [ ] Supported-platform binaries
- [ ] Documentation
- [ ] Compatibility policy

## Future Direction

Potential future capabilities include persistent AI-agent memory, self-describing collections, MCP integration, and semantic retrieval.

These are exploratory features and are not part of the initial release.

## Contributing

ZylonDB is an early-stage open-source project. Contributions, discussions, bug reports, and architectural feedback are welcome.

Contribution guidelines will be added as the initial implementation takes shape.

## License

ZylonDB is intended to be dual-licensed under MIT OR Apache-2.0.