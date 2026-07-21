# Agent Instructions

**Project name:** shimesu

## Scope and precedence

These instructions apply to the entire repository unless a more specific `agents.md` exists in a subdirectory. Follow the repository's existing conventions first when they are compatible with this document. Direct task instructions may refine the implementation, but do not silently weaken security, data-safety, or cost constraints.

Use **MUST**, **SHOULD**, and **MAY** in their usual normative sense.

## Product definition

### Elevator pitch

This project is an open-source, self-hostable static publishing platform for AWS. A user brings an AWS account and a domain, deploys two coordinated CloudFormation stacks, and then uses a simple CLI to publish static artifacts to site-specific subdomains. It should make the useful parts of S3, CloudFront, DNS, TLS, and basic traffic reporting accessible to humans and AI agents without requiring them to assemble or operate those services manually.

Typical content includes:

- personal sites and landing pages;
- one or many HTML files;
- generated reports, presentations, plans, and visualizations;
- Markdown and other static documents;
- arbitrary static assets such as CSS, JavaScript, images, fonts, and downloads.

The product hosts static files. It is not a general application runtime, build platform, or serverless backend.

### Core product promise

A user should be able to move from a local file or directory to a stable HTTPS URL with one predictable command, while retaining ownership of the AWS account, domain, data, and bill.

The system must be:

1. **Self-hosted and user-owned.** Resources run in the user's AWS account.
2. **Cheap at rest.** Idle infrastructure should have no meaningful recurring compute cost. Charges should primarily follow storage, requests, traffic, logs, and explicit queries.
3. **Simple.** Prefer a small number of managed AWS resources and a direct CLI workflow.
4. **Safe.** Publishing, updating, and deleting content must not risk unrelated resources or data.
5. **Automation-friendly.** Every important workflow must work non-interactively and expose stable machine-readable output.
6. **Open-source first.** Do not introduce a required hosted control plane or proprietary dependency. Future commercialization must remain possible without compromising the self-hosted edition.

## Vocabulary

Use these terms consistently:

- **Installation:** One deployed instance of the product in an AWS account, represented by two CloudFormation stacks: a certificate stack always deployed to `us-east-1`, and a regional data-plane stack deployed to the operator's chosen region.
- **Base domain:** The domain under which sites are published, such as `pages.example.com`.
- **Site:** A named static site with a stable DNS-safe slug and subdomain, such as `report.pages.example.com`.
- **Deployment:** A published snapshot or revision of a site's static files.
- **Control plane:** The CLI and the AWS API operations it performs.
- **Data plane:** The AWS resources that store and serve published files.

## Current phase and committed MVP decisions

Treat the following as current product decisions, not suggestions:

- The MVP is **single-user** and intended for one operator or one trusted AWS environment.
- The MVP is **CLI-first**. Do not build a web interface unless a task explicitly changes the scope.
- The CLI uses the standard AWS credential provider chain and talks directly to AWS APIs.
- The MVP does **not** need a product-owned HTTP API, API Gateway, long-running service, or control-plane Lambda.
- Infrastructure is provisioned through CloudFormation and should be reproducible without manual console work.
- Each site receives its own subdomain beneath the installation's base domain.
- Publishing supports both a single file and a directory tree.
- Published content is served over HTTPS.
- The default hosting path is static-only. Do not execute uploaded code on the server.
- AWS is the only required cloud target for the MVP. Do not add a generic multi-cloud abstraction prematurely.
- Basic deployment visibility is in scope: users should be able to discover what sites exist, where they are available, and what was most recently deployed.
- Traffic statistics are desirable, but the core publish path takes priority. Do not delay a working publisher to build sophisticated analytics.

## Settled implementation decisions

These decisions are reflected in the current codebase. Treat them as constraints. Reopening any of them requires an explicit ADR.

- **Language:** Rust, edition 2021. MSRV 1.94.1 as declared in `Cargo.toml` and required by the current AWS SDK dependency graph.
- **CLI framework:** `clap` v4 with the derive API. Binary name is `shimesu`, entry point `src/main.rs`.
- **Async runtime:** `tokio` with the `full` feature set.
- **AWS SDK:** `aws-sdk-*` crates (v1.x). Load configuration with `aws_config::defaults(BehaviorVersion::latest())`. Never construct credentials or region values manually.
- **Error handling:** `thiserror` for typed domain errors (`ShimesuError`); `anyhow` for top-level propagation. Preserve the original AWS SDK error as structured diagnostic context while surfacing a concrete, actionable user-facing message.
- **Output:** `owo-colors` for terminal color, `indicatif` for progress, `dialoguer` for interactive prompts. `--json` or non-TTY stdout selects machine-readable output; JSON mode writes only the result to stdout and diagnostics to stderr. Human-mode status and progress may use stdout while warnings and errors use stderr. Keep human-readable and JSON output branches separate — do not embed display logic in data structs.
- **Config file format and path:** TOML at `~/.config/shimesu/config.toml` (located via the `dirs` crate) with an `[installation]` table (`stack_name`, `region`, `profile`). Precedence: CLI flag > environment variable > config file > hardcoded default. The env var `SHIMESU_STACK` controls the stack name.
- **Command names and subcommand structure:** `status`, `stack init`, `stack status`, `stack update`, `stack destroy`, `stack teardown`, `doctor`, `publish <path>`, `site list`, `site inspect <slug>`, and `site delete <slug>`. A first publish creates the site record and later publishes update it. There is no separate site-creation or deployment-history command. These are stable public API; do not rename without a documented breaking change.
- **Site metadata store:** DynamoDB, `PAY_PER_REQUEST` billing. Table name is `{stack_name}-projects`. Hash key `slug` (String). This is the sole authoritative source for site records — do not duplicate metadata in S3.
- **Provisioning model:** Two CloudFormation stacks. `infra/certificate.yaml` is always deployed to `us-east-1` (CloudFront certificate requirement); `infra/stack.yaml` deploys the data plane to the operator's chosen region. The `CertificateArn` output of the first stack is a required parameter of the second.
- **Content prefix layout in S3:** Each site occupies `{slug}/` in the shared bucket. The reserved prefix `_shimesu/` holds the base domain landing page. No site slug may start with `_shimesu`. A single-file publish stores the file as `{slug}/index.html`; directory and zip publishes preserve their relative tree under `{slug}/`.
- **Retention policy:** The S3 bucket, DynamoDB table, and managed ACM certificate carry `DeletionPolicy: Retain` and `UpdateReplacePolicy: Retain`. Stack deletion must never destroy user content, site records, or operator-supplied certificate overrides. Explicit teardown may remove only resources whose exact Shimesu ownership tags match the selected installation.

## Explicit non-goals for the MVP

Do not add these without an explicit scope change:

- multi-user accounts, organizations, roles, invitations, or billing;
- a hosted SaaS control plane;
- a web dashboard;
- Cognito or another application login system;
- API Gateway solely to proxy operations the CLI can perform with AWS credentials;
- dynamic application hosting, server-side rendering, functions supplied by users, or arbitrary code execution;
- a Git provider integration, CI/CD platform, or build service;
- a visual editor or content-management system;
- custom domains per site beyond the installation's site subdomains;
- real-time analytics pipelines;
- multi-cloud support;
- containers, virtual machines, relational databases, search clusters, or always-on workers.

## Preferred starting architecture

Use this as the default architecture. A materially different design requires a short architecture decision record explaining cost, security, operability, and migration consequences.

### Provisioning

- Use two CloudFormation stacks: `infra/certificate.yaml` always in `us-east-1` for the ACM certificate, and `infra/stack.yaml` in the operator's chosen region for the data plane. The `CertificateArn` output of the first stack is a required parameter of the second. Do not collapse these into one stack.
- Accept explicit domain and hosted-zone inputs rather than guessing across accounts.
- Prefer an MVP assumption that DNS is managed in Route 53 in the same AWS account. Supporting external DNS providers can come later; when external DNS is unavoidable, output the exact records the user must create.
- Expose stable stack outputs for every value the CLI needs. The CLI should consume outputs rather than duplicate resource-discovery logic or hard-code generated names.
- Resource names must be account-safe and collision-resistant. Never require a globally unique bucket name chosen manually by the user.
- Tag resources consistently so users can identify ownership and cost.
- Retain user content on ordinary stack deletion unless the user invokes a separate, explicit destructive teardown flow.

### Static delivery

Prefer a minimal shared data plane:

- a private S3 bucket for published content;
- S3 public-access blocking enabled;
- CloudFront in front of S3 using origin access control or the current recommended private-origin mechanism;
- ACM-managed TLS;
- Route 53 records for the base domain and site subdomains;
- one shared distribution and wildcard routing where this remains simpler and cheaper than one distribution per site;
- a lightweight edge rewrite only when needed to map a request host to a site prefix.

Do not create a Lambda merely because the architecture is described as “serverless.” Use Lambda only for a concrete computation that cannot be handled safely by the CLI, CloudFormation, S3, CloudFront, or a lighter edge mechanism.

### Metadata

Start with the least complex durable source of truth.

- Prefer a versioned installation or site manifest in S3 when the access pattern is simple lookup and listing.
- Add DynamoDB only when there is a demonstrated need for indexed queries, conditional writes, concurrency control, or scale that an S3 manifest cannot handle cleanly.
- Do not maintain the same authoritative metadata independently in multiple stores.
- Every stored schema must include a version and have an upgrade path.

A site record should be capable of representing at least:

- site ID and DNS-safe slug;
- canonical URL;
- creation and update timestamps in UTC;
- current deployment identifier or manifest;
- optional display metadata;
- schema version.

### Analytics

Use AWS-native request data and favor on-demand work over continuous processing.

- Aggregate CloudFront metrics may be used for installation-level health or traffic.
- Per-site statistics should be derived from request logs or another source that can distinguish the requested host or site path.
- Prefer standard logs stored in S3 and on-demand local or query-based aggregation before introducing streaming ingestion.
- Do not add Kinesis, OpenSearch, an always-running collector, or scheduled compute for the MVP.
- Define “view” precisely. A request is not necessarily a unique human view, and cached, bot, or asset requests must not be presented as exact visitor counts.
- Statistics output must disclose the time range, aggregation method, source delay, and whether numbers are estimates.
- Log retention and privacy implications must be documented and configurable.

## Cost rules

Near-zero idle cost is a product requirement.

Agents MUST NOT introduce the following without explicit approval and a written cost justification:

- NAT Gateways;
- EC2, ECS services, Fargate tasks, or Kubernetes;
- RDS, OpenSearch, ElastiCache, or provisioned database capacity;
- always-on background processes;
- frequent scheduled jobs;
- one CDN distribution, certificate, bucket, or database per site when a shared resource can isolate sites safely;
- third-party services required for normal operation.

For every new AWS service or recurring process, document:

1. why existing resources cannot satisfy the requirement;
2. expected idle cost;
3. primary usage-based cost drivers;
4. how the user can disable or remove it;
5. what data it stores and for how long.

Prefer pay-per-request or on-demand modes. Cost optimizations must not make deletion unsafe, obscure billing, or create unrecoverable state.

## CLI contract

The CLI is both the human interface and the initial AI interface. Design it as a stable product surface, not a thin collection of scripts.

### Required behavior

The CLI should eventually cover these capabilities, although exact command names may follow repository conventions:

- provision or update an installation;
- inspect installation status and stack outputs;
- create, list, inspect, and remove sites;
- publish a file or directory to a site;
- show the site's canonical URL and deployment state;
- list deployments or at least report the current deployment;
- inspect basic traffic statistics when analytics are available;
- diagnose credentials, region, domain, stack, and permissions.

### Interface rules

- Use the standard AWS profile, region, environment-variable, workload-identity, and credential-chain behavior. Never store access keys.
- Provide explicit `--profile` and `--region` overrides where the AWS SDK does not already handle them clearly.
- Commands must be non-interactive when all required arguments are supplied.
- All read operations and successful write operations SHOULD support stable `--json` output.
- In machine-readable mode, write only the result to stdout. Send diagnostics to stderr.
- Exit with zero only when the requested operation succeeded. Use documented nonzero exit codes or structured error categories.
- Prompts must never appear in non-interactive or JSON mode.
- Destructive commands require a clear confirmation in an interactive terminal and an explicit flag for automation.
- Errors must identify the failed resource or operation and provide a concrete recovery action without exposing secrets.
- Support AWS pagination, throttling, retries, and partial failures deliberately; do not assume small accounts or perfect networks.
- Use UTC and ISO 8601 timestamps in stored data and JSON output.
- Keep JSON field names and meanings backward compatible. Add fields rather than silently changing semantics.
- Do not add anonymous telemetry by default. Any future telemetry must be opt-in and documented.

### Site and path safety

- Site slugs must be valid DNS labels and must be normalized or rejected deterministically.
- Never allow a site slug, path, manifest entry, or symlink to escape its assigned S3 prefix or local source root.
- Reject ambiguous path traversal, absolute paths, control characters, and unsafe object keys.
- Do not follow symlinks outside the selected publish root.
- A deletion or prune operation must prove that its target is inside the exact installation and site namespace before issuing AWS delete calls.
- Never use an unbounded bucket-wide delete to update one site.

### Publishing semantics

- Upload files byte-for-byte unless the user explicitly requests a transformation.
- Do not require a build step. The tool publishes build outputs; it is not the build system.
- Detect and set appropriate content types. Preserve binary files correctly.
- Set cache metadata intentionally. Long-lived caching is appropriate for immutable or content-addressed assets; entry documents should remain updateable without excessive invalidations.
- Compare hashes or equivalent metadata to avoid re-uploading unchanged files.
- Make repeated publication of the same input idempotent.
- Define deployments as complete snapshots by default, not accidental additive uploads. Stale files should be pruned only within the site's namespace and according to documented semantics.
- Write or update the deployment manifest only after required uploads succeed.
- If publication is not fully atomic, make the limitation explicit and order operations to minimize broken intermediate states.
- Treat CDN invalidation and DNS propagation as asynchronous. Distinguish “uploaded successfully” from “confirmed available at every edge.”
- Print the canonical URL after a successful publish.

## Security and privacy

Security defaults are not optional, even for a single-user MVP.

- Keep content buckets private and serve them through the CDN.
- Enforce HTTPS and modern TLS through managed AWS services.
- Use least-privilege IAM permissions for the CLI and all provisioned resources.
- Never log AWS credentials, session tokens, signed URLs, secret environment values, or full sensitive request headers.
- Treat uploaded HTML and JavaScript as untrusted content. Do not evaluate it in the CLI or control plane.
- Keep any future management interface on an origin separate from hosted site content. Never rely on parent-domain cookies shared with untrusted site subdomains.
- Validate all CloudFormation parameters and CLI inputs before using them in resource names, DNS names, IAM policies, shell commands, or object keys.
- Avoid shelling out to AWS CLI commands when the SDK offers a safe API. When a subprocess is necessary, pass arguments without shell interpolation.
- Restrict cross-origin access by default. Do not add permissive CORS policies unless a documented use case requires them.
- Make log collection visible to the operator. Document that access logs may contain IP addresses, user agents, referrers, and requested paths.
- Prefer data retention over automatic deletion. Permanent removal must be explicit, scoped, and testable.
- Do not broaden IAM permissions to `*` merely to make development easier. If a wildcard is unavoidable for an AWS API, constrain actions, conditions, account, region, and resource scope as far as AWS permits and explain the exception.

## Reliability and upgradeability

- CloudFormation deployments and CLI operations must be idempotent.
- Use deterministic manifests and stable identifiers.
- Handle AWS throttling with bounded retries and jitter, preferably through SDK-native retry behavior.
- Do not hide partial success. Report uploaded, skipped, failed, and deleted counts accurately.
- Preserve enough state to diagnose an interrupted deployment.
- Make stack upgrades backward compatible with existing content and site records.
- Never replace or delete a stateful resource solely because its generated name or template layout changed.
- Introduce schema migrations explicitly. Readers should tolerate the previous schema version during a safe transition where practical.
- Provide clear behavior when a stack is missing, drifted, updating, rolled back, or in a failed state.
- Make cleanup bounded and resumable. A failed cleanup must not leave the installation metadata claiming that data is gone.

## Open-source and future-commercialization constraints

- The self-hosted core must function without calling a vendor-operated service.
- Keep cloud operations behind small internal boundaries so a future hosted control plane can reuse behavior without forking the core.
- Do not add speculative enterprise abstractions to the MVP.
- Avoid hard-coded vendor accounts, domains, telemetry endpoints, or license checks.
- Check new dependency licenses for compatibility with the repository's license and document major runtime dependencies.
- Prefer maintained, widely used libraries only when they remove meaningful complexity. Small functionality should not bring a large dependency tree.
- Keep persistent formats documented and portable enough that users can inspect and recover their own data.

## Engineering workflow for agents

### Toolchain

```bash
cargo build                                              # compile
cargo test                                               # run all tests (no AWS account needed)
cargo clippy --all-targets --all-features -- -D warnings # lint
cargo fmt --check                                        # format check
cargo run -- <command> [args]                            # run CLI from source
```

Add dependencies by editing `Cargo.toml`. Prefer narrowly scoped feature flags. Do not bump the MSRV (`rust-version = "1.94.1"`) without a deliberate decision and a documentation update.

### Before modifying code:

1. Read the relevant source, tests, infrastructure templates, and documentation.
2. Identify whether the change affects persistent data, IAM, DNS, TLS, costs, deletion behavior, or public CLI output.
3. Choose the smallest coherent implementation that satisfies the current phase.
4. Reuse existing abstractions and conventions before creating new ones.
5. For a material architectural deviation, add or update a short ADR before or with the implementation.

While modifying code:

- Keep changes narrow. Do not mix unrelated refactors with feature work.
- Prefer clear, typed domain models and explicit dependency injection for AWS clients.
- Keep AWS-specific calls out of parsing and core decision logic so the latter can be unit tested without an account.
- Avoid hidden global state and import-time network calls.
- Validate inputs at system boundaries.
- Preserve the original underlying AWS error as structured diagnostic context while presenting an actionable user-facing message.
- Do not silently create expensive resources, enable logging, or change retention settings.
- Do not manually edit generated files. Update the source and regenerate them using the documented command.
- Do not commit secrets, account IDs from real environments, generated credentials, local state, or deployment artifacts.

After modifying code:

1. Run the smallest relevant checks, then the full local test suite when practical.
2. Update command help, README examples, configuration references, and architecture documentation affected by the change.
3. Verify both human-readable and JSON output when a CLI command changes.
4. Verify upgrade and teardown behavior when infrastructure changes.
5. Summarize behavior changes, important design choices, tests run, and unresolved risks.

When a requirement is ambiguous, prefer the option that is more reversible, uses fewer AWS services, has lower idle cost, and preserves user data. Record the assumption rather than expanding scope silently.

## Testing expectations

The default test suite must not require a real AWS account or incur cloud costs.

**Rust-specific guidance:**
- Use `assert_cmd` and `predicates` (already in dev-dependencies) for CLI integration tests that invoke the compiled binary.
- Use `tempfile` for temporary directories in tests that touch the filesystem.
- Abstract AWS service calls behind traits or thin wrapper structs so unit tests can substitute fake implementations without a network. Keep the trait boundary at the crate boundary, not buried inside business logic.
- Do not use `#[ignore]` to skip tests that fail locally — fix the test or gate it behind a feature flag with documentation.
- Test modules that need environment-variable isolation must serialize access with a `Mutex` (see the pattern in `src/config.rs`) because `std::env` is process-global.

Include tests at the appropriate levels:

- unit tests for slug validation, path handling, manifests, content metadata, command parsing, and error mapping;
- unit tests with stubbed or fake AWS clients for pagination, retries, partial failures, and permission errors;
- template validation and policy checks for CloudFormation and IAM;
- tests proving that content buckets are not public;
- tests proving that delete and prune operations cannot escape a site prefix;
- deterministic snapshot or golden tests for stable JSON and human-readable CLI output where useful;
- opt-in integration tests against an isolated disposable stack;
- an end-to-end smoke path covering provision, publish-driven site creation, fetch over HTTPS, update, inspect, and safe teardown.

Integration tests must use unique names, tag every resource, expose a cleanup command, and avoid running automatically in ordinary pull requests unless a dedicated environment is configured.

## Documentation requirements

The repository should make the following discoverable without reading source code:

- the elevator pitch and current limitations;
- prerequisites, including AWS credentials and domain/DNS assumptions;
- the quickest path from an empty AWS account setup to a published URL;
- all AWS resources created and why they exist;
- expected cost drivers and how to disable optional features;
- the security model and IAM permissions;
- publish, update, inspect, statistics, and teardown examples;
- data retention and deletion behavior;
- analytics definitions and privacy implications;
- how to recover or access content without the CLI;
- the status of experimental features and breaking changes.

Examples must be copy-pasteable, must not contain real account identifiers or secrets, and should show both human and JSON workflows where relevant.

## Preferred implementation sequence

Unless a task requires a different order, prioritize a thin vertical slice:

1. CloudFormation provisioning for the minimal private static-delivery path.
2. CLI installation discovery and diagnostics using normal AWS credentials.
3. Publish-driven site creation, listing, and inspection.
4. Publishing a file or directory and obtaining a working site URL.
5. Safe update, prune, inspect, and site removal behavior.
6. Stable JSON output and agent-friendly non-interactive workflows.
7. Low-cost, clearly defined traffic statistics if justified by demand.
8. Deployment history or rollback only after its persistent schema and cost are justified.
9. A web interface only after the CLI and resource model are stable.

## Decisions intentionally left open

Do not treat these as settled unless the repository contains an ADR or a direct task decides them:

- exact deployment versioning and rollback strategy;
- precise log format, query mechanism, and retention defaults for statistics;
- support for domains outside Route 53 or hosted zones in another account;
- custom domains per site;
- Markdown rendering versus serving Markdown as a static file;
- a future web UI, API, MCP server, or hosted commercial control plane.

For an open decision, implement only what the current task requires. Favor a small, documented, reversible choice over a broad abstraction.

## Definition of done

A change is complete only when all applicable statements are true:

- It advances the CLI-first, self-hosted static-publishing use case.
- It does not introduce an unapproved always-on or high-idle-cost component.
- It preserves least-privilege access and private-origin hosting.
- It cannot delete or overwrite data outside the intended installation and site scope.
- It behaves predictably in both interactive and automated use.
- It handles AWS errors and partial failure honestly.
- It includes relevant tests that run without AWS by default.
- It updates affected documentation and public CLI output contracts.
- It accounts for existing installations, persistent data, upgrade behavior, and teardown behavior.
- Any new cost, retained data, privacy impact, or architectural tradeoff is documented.
