# Security model

Tesela assumes the embedding host authenticates principals and owns transport security, secrets, encryption, tenant isolation, and durable audit storage. The runtime enforces semantic authorization inside that trusted boundary.

For every operation Tesela checks cancellation and deadlines, validates the immutable delegated-authority chain, applies bounded resource counters, evaluates host policy, validates semantic input and preconditions, then calls the bound backend or executor. A child authority grant cannot broaden scope, expiry, call budget, or delegation depth. An empty authority chain is only appropriate at a trusted host boundary.

Policy can deny, add row predicates, redact properties, constrain action input, require approval, transform execution, and attach obligations. These restrictions apply to `get`, search, traversal sources and targets, aggregation inputs/group keys, actions, and capability discovery. Authority is checked before policy so policy cannot enlarge delegated authority.

Mutations use backend-native conditional operations. Revision mismatch and semantic-precondition failure report `not_started`; Tesela never implements upsert as `get` followed by create/update and never emulates an atomic batch. Action effects, approval mode, concurrency contract, idempotency metadata, revisions, and preconditions are part of the compiled contract, while executable code remains an immutable host binding.

Observers cannot fail an already committed operation. Hosts needing durable audit must write an outbox record in the same transaction as their backend mutation, then observe Tesela receipts for correlation. Receipts contain request, principal, ontology identity, semantic resource, source, redactions, obligations, declared effects, approval state, and latency; they must be handled as potentially sensitive operational data.

`AllowAllPolicy` and `tesela::dev::MemoryBackend` are development utilities. Production hosts should supply explicit policy and backend implementations and run the backend conformance suite.

Report suspected vulnerabilities privately to the repository maintainer with an affected version and minimal reproduction before opening a public issue.
