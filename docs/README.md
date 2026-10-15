# Documentation & Dossiers d'Architecture pydbsrt

Index chronologique des rapports d'audit, benchmarks et études d'architecture :

1. [2026-10-15-audit-global.md](file:///home/latty/Prog/__PERSO__/pydbsrt/docs/2026-10-15-audit-global.md) : Audit exhaustif du dépôt (code Python, tests, CI, sous-modules, Docker, sécurité et obsolescence).
2. [2026-10-15-modernization-blueprint.md](file:///home/latty/Prog/__PERSO__/pydbsrt/docs/2026-10-15-modernization-blueprint.md) : Feuille de route de modernisation globale (UV, Taskfile, PostgreSQL 17, réécriture Rust).
3. [2026-10-15-pg17-bktree-benchmark-migration.md](file:///home/latty/Prog/__PERSO__/pydbsrt/docs/2026-10-15-pg17-bktree-benchmark-migration.md) : Benchmark comparatif PG11 BK-Tree vs PG17 pgvector HNSW et validation de l'extension BK-Tree sur PG17.
4. [2026-10-15-scaling-architecture-rust-mmap.md](file:///home/latty/Prog/__PERSO__/pydbsrt/docs/2026-10-15-scaling-architecture-rust-mmap.md) : Étude d'ingénierie sur le passage à l'échelle (milliards de frames), analyse des limites Python et conception d'un moteur Rust `mmap` Zero-Copy.
5. [2026-10-15-rust-e2e-benchmarks-and-tradeoffs.md](file:///home/latty/Prog/__PERSO__/pydbsrt/docs/2026-10-15-rust-e2e-benchmarks-and-tradeoffs.md) : Rapport d'ingénierie Rust E2E (`dbsrt-rs`), benchmarks (15x à 1 200x) et analyse critique des inconvénients/trade-offs.
