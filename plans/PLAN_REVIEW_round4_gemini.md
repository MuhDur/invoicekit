Warning: True color (24-bit) support not detected. Using a terminal with true color enabled will result in a better visual experience.
Ripgrep is not available. Falling back to GrepTool.
Here are the remaining structural breaks in v0.5. The plan is approaching steady-state, but execution dependencies are currently broken in a few load-bearing places.

### 1. Cassette Framework Ordering (T-074 chain)
- **Bug**: T-074 (`transmit-mock`) demands that "the recorder produces [cassettes] deterministically". But the recorder isn't built until T-074a. T-074 depends on T-070, bypassing T-074a entirely.
- **Fix**: 
  `- T-074 | Mock gateway... | Depends on: T-070`
  `+ T-074a | Cassette recorder... | Depends on: T-070, T-120`
  `+ T-074 | Mock gateway... | Depends on: T-074a`

### 2. Signer Architecture Overlap (T-083b vs Track 8)
- **Bug**: T-083b (Track 6) assigns the ZATCA and CFDI signer adapters to a core agent. However, Track 8's `report-sa-zatca` *also* tasks the country-crate agent with "ECDSA secp256k1 signing... built as cryptographic archetype". 
- **Fix**: Remove the cryptography implementation from Track 8 and make those country crates explicitly dependent on T-083b. Two agents cannot build ZATCA cryptography simultaneously.

### 3. Missing Phase 2.5 (Feasibility Manifests) in Section 6
- **Bug**: §3.3 declares that Phase 2.5 (Country feasibility manifests) is a hard prerequisite before any Phase 3 country crate starts. But Section 6 (Build Sequence) contains **zero tasks** for Phase 2.5.
- **Fix**: Add a dedicated Track (e.g., Track 7.5) with tasks to generate the ~60 manifests, including sourcing the baseline cassettes. Track 8 must depend on these manifest tasks.

### 4. Timeline/Math Contradiction: Archetypes vs Wave 1
- **Bug**: §3.3 states Archetype lock-in runs *strictly sequentially* across weeks 14–27, and ZATCA is an archetype taking 6–8 weeks. But Track 8 lists ZATCA *inside* Wave 1, and says Wave 1 takes "weeks 14–18".
- **Fix**: A 6-8 week sequential prerequisite cannot fit inside a 4-week parallel wave. 
  `- Wave 1 — Regulatory urgency (weeks 14–18 if parallel) | report-sa-zatca | 6-8 weeks`
  `+ Archetype Lock-in (weeks 14-24, sequential) | report-sa-zatca | 6-8 weeks`
  Move the three archetypes out of the "Waves" entirely. The Waves should strictly represent the parallel work that happens *after* lock-in.

### 5. Capability Matrix Terminology Drift
- **Bug**: The term "Capabilities" is dangerously overloaded.
  - §3.4 uses it for business maturity (`partner-live, archive, correction`).
  - T-033 uses it for technical runtime constraints (`local_validate, requires_service`). 
  - §9 invents two new terms out of thin air: `serialize-alpha` and `validate-beta`.
- **Fix**: Rename T-033 to `Execution Context Matrix` or `Validator Runtime Matrix`. Strip `serialize-alpha/validate-beta` from §9 and use the actual labels defined in §3.4.

### 6. Track 8 Missing Task IDs & Acceptance Criteria
- **Bug**: Tracks 0-7 and 9-11 use formal `T-XXX` task IDs. Track 8 drops them entirely and just lists crate names (`report-it-sdi`). §8.1 mandates that every country task arrives with "Done criteria", but Track 8 provides none.
- **Fix**: Assign formal task IDs (e.g., `T-801 report-it-sdi`) so the agent `br` dependency graph works. Point the acceptance criteria to the outputs of the missing Phase 2.5 manifest tasks.

### 7. Phase Numbering Confusion
- To answer your specific question: **`0.5` is the document version** (`Implementation Plan v0.5`), not a phase. The text correctly uses Phases 0, 1, 2, 2.5, 3-7. The phase taxonomy is fine, but the *mapping* of those phases into executable Section 6 Tracks is what is currently broken (see Bug #3 above).
