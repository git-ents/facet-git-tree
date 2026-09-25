# Prompt: Independent Technical Evaluation of a Database Architecture Proposal

## Role

You are a senior database researcher and systems engineer — the CIDR-paper-reviewer register. Twenty years across LSM storage engines, MVCC concurrency, distributed consensus, and lakehouse table formats. Your tone is blunt, quantitative, and taxonomy-first. You do not flatter, and you do not hedge to be polite. Where the design is right, you say so in one sentence and move on. Where it is wrong, overstated, or unproven, you show the failure mechanism, not just the verdict.

You judge three things independently: **soundness** (do the invariants hold?), **novelty** (what is actually unexplored vs. prior art?), and **shipability** (would this survive production?). A design can score high on one and zero on the others.

## The proposal under evaluation

### 1. Thesis

Build a general-purpose database as an append-only log whose storage and caching semantics mimic Git's object model: all facts are immutable content-addressed objects; all derived structures are rebuildable caches; all mutability is concentrated in a tiny, CAS-serialized frontier of named pointers; deletion is reachability expiry, not mutation. Locally, a node caches (or lazily fetches) everything; remotely, a cold store holds the record under opt-in rolling retention. Candidate workloads: agent action logs, code, compilation outputs, CI artifacts — engineering telemetry broadly.

### 2. The four invariants

1. **Identity is content.** Every node at every tier is addressed by the hash of its encoding. Cache coherence is structural: entries cannot be stale, only unreachable.
2. **Writes never mutate.** Updates append nodes and rewrite only the path from root to change (copy-on-write trie/Merkle DAG).
3. **One tiny mutable frontier.** All mutability is a small set of named pointers swapped by compare-and-swap. Readers are lock-free snapshots; multi-writer convergence is pointer CAS plus three-way DAG merge with explicit conflicts.
4. **Eviction is reachability, not TTL.** Derived structures are evictable caches; primary facts persist until a reachability policy prunes them. Deletion is ref-granular: refs partition facts by retention scope; expiring a partition ref and running GC deletes its facts.

### 3. Architecture

- Hot local node: append-only object store, fully durable on write (fsync + ref advance). May cache everything (full clone) or only the working set (partial clone; missing objects are legal dangling references).
- Cold remote record: append-only WORM store. Local→remote is asynchronous write-back; consistency is ref-level CAS; conflicts resolve via three-way merge or explicit refusal.
- Promisor/lazy fetch makes caching a policy dial — "cache everything" to "fetch on demand" — with identical correctness. `materialize(oid)` on a pruned object fails with a distinguishable **Expired** state.

### 4. Evidence gathered

Method: stock Git, two simulations of "one agent minute" (12–60 actions; one commit per action). Measured object and pack sizes directly.

| Case | Raw bytes/action | Packed bytes/action |
|---|---|---|
| Incompressible observation payload | ~2.7 KB | ~1.7 KB |
| Real tool output (highly similar across actions) | ~4.9 KB | **~0.76 KB** |

Pack delta compression dominated: agent loops re-read the same files, so observation blobs dedup ~6×. Extrapolation: ~15–35 KB packed per agent-minute; ~1–2 GB per agent-year.

Cost model for a 100-engineer org, rolling retention: code history 50–200 MB/day at ∞ retention; agent logs ~0.5 GB/day at 30–90 d; build CAS 5–50 GB/day at 7–30 d; CI logs 0.5–2 GB/day at 30 d. Headline: ~5–50 GB/day of cold storage, bounded at ~1–2 TB steady state, ≈ $10–40/month at cold-tier pricing.

### 5. Claims vs. assumptions

**Claims:**
- C1. Cache-coherence disappears under content addressing.
- C2. Per-action storage cost is structure-dominated and structure dedups to near-nothing (measured, §4).
- C3. Deletion via ref expiry + reachability GC is exact and O(refs).
- C4. Multi-writer convergence is CAS + three-way DAG merge; conflicts are explicit, refusals are safe.

**Assumptions (unverified):**
- A1. Copy-on-write write amplification stays acceptable for hot-path workloads, not just append-heavy ones.
- A2. Optimistic CAS on the pointer frontier provides adequate multi-writer throughput at agent-scale concurrency.
- A3. Derived-index staleness (read-validate-and-fall-back) is acceptable in place of transactional index updates; unique constraints degrade gracefully to optimistic verify-and-CAS.
- A4. The Expired-fact contract (pruned vs. corrupt vs. never-existed) can be enforced at every read path, including third-party Git tooling.
- A5. Pins/attestations binding claims to OIDs can be modeled as a reachability extension without pathological retention coupling.

### 6. Known limits (claimed)

Not competitive with in-place B-tree databases for hot OLTP. SHA-1 OIDs, no self-describing wire types unless added above the substrate. Binary artifacts pack poorly. GC policy spanning local+remote is the crown-jewel design risk.

### 7. Claimed precedents

Dolt, Noms, Datomic, IPFS/IPLD, Bazel Remote CAS. Claimed differentiator: Git's pack/GC/transport + partial-clone promisor semantics used as a retention dial, with ref-expiry as deletion.

### 8. Amendments added after a first review round (evaluate the amended design, not just the original)

- **A. Codec tiering with embedded version.** Deterministic, versioned encodings; one codec version per tier; the codec epoch is a leaf inside each entity, so encoding is part of content and identity survives migration.
- **B. Migration-as-facts.** Codec migrations are pure functions recorded as facts in the log; the migrated corpus is a derived, rebuildable cache; compaction = periodic re-encode into the current codec, with the old codec partition expiring as just another retention scope.
- **C. Tombstones.** Deletion and pins expressed as facts. Tombstones bind to entity identity + version in a naming layer above OIDs (content cannot be deleted by hash, only facts by name); liveness = reachability **minus negation**; tombstone-propagation state and the pin registry remain non-content metadata.

## Your evaluation tasks

1. **Taxonomy translation.** Restate the design in standard database vocabulary (LSM, MVCC, compaction, register semantics, tracing GC). Then state the consistency contract precisely: isolation level of reads, write serializability or its absence, session guarantees. Do not let informal language ("readers are lock-free snapshots") survive unaudited.
2. **Claim adjudication.** For each of C1–C4: supported, overstated, or refuted — with the mechanism.
3. **Assumption stress.** For each of A1–A5: quantify the breakpoint where possible (e.g., the write-amplification crossover in updates-per-key-per-flush-window), and say what evidence would settle it.
4. **Missing invariants.** Is the four-invariant set complete? Identify any load-bearing requirement the design silently assumes (consider: encoding determinism, non-content side metadata and its consistency, durability semantics of the async write-back window, naming/authority).
5. **Answer the proposer's open questions.** (a) Is the invariant formulation complete? (b) Where does write amplification actually break? (c) Is ref-per-retention-partition the right deletion granularity, including for downstream-reachability-dependent retention? (d) Does the Expired error contract survive real consumers (diff tools, CI, graph walkers) that treat absence as corruption? (e) What does the scheme not model that a general database must (secondary indexes under contention, cross-repo atomicity, row-level authorization, schema evolution)? Each with a one-sentence verdict plus argument.
6. **Prior art map.** Group precedents by tradition — systems/substrate, databases, table formats/analytics, academic research. For each: what it proves, what it left open, and what it implies for this design's novelty claim. Include at least one pre-Git content-addressed storage system. Then judge §7's differentiator honestly: what exactly, if anything, is unexplored?
7. **Amendment critique.** Do the codec/migration/tombstone amendments close the gaps they claim to close, or merely relocate the problem? Be precise about what residual non-content mutable state remains.
8. **Falsification plan.** Design the experiments that would settle the open assumptions: deterministic simulation, Jepsen-style testing of the ref CAS and GC, baselines against RocksDB and a lakehouse table format, write-amplification curves, p99 read latency including remote miss paths, and a steady-state measurement protocol that reflects realistic push cadence (not single-batch packing). For each experiment, state what result falsifies the design.
9. **Verdict.** Score soundness, novelty, and shipability each 0–10 with ≤3 sentences of justification. Recommend one of: research paper, niche product, or dead on arrival — and name the single gating risk. End with the three strongest and three weakest points of the proposal.

## Standards

- Distinguish "wrong" from "overstated" from "unproven" — never collapse them.
- Quantify. Give numbers with stated assumptions whenever you can; mark estimates as estimates.
- Name systems, papers, and mechanisms precisely; cite nothing you cannot describe.
- Every criticism must include its failure mechanism; verdicts without mechanisms are worthless.
- Do not summarize the proposal back to the reader. Evaluate it.
- Format: markdown, comparative tables where they earn their space. Length target 1,500–2,500 words.
