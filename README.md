# shimesu

![shimesu hero image](https://raw.githubusercontent.com/michidk/shimesu/main/.github/images/hero.png)

**shimesu** (示す, "to show") is an open-source, self-hosted static publishing platform for AWS, built for AI-generated artifacts. When an AI agent produces a report, presentation, plan, visualization, or any static output, one command gives it a stable HTTPS URL — while the AWS account, domain, content, and bill remain yours.

```console
$ shimesu publish ./report.html --yes
✓ Published site 'report'
URL: https://report.static.example.com
Deployment: 01EXAMPLEDEPLOYMENT
Files: 1
Uploaded: 1
Skipped: 0
Deleted: 0
```

Typical artifacts: AI-generated reports and dashboards, slide decks, data visualizations, Markdown documents, personal sites, and arbitrary static assets. Every command is non-interactive and machine-readable by design, so AI agents can publish, inspect, and remove sites without human intervention.

There is no hosted control plane, application server, build service, or required third-party runtime. The CLI uses the standard AWS credential provider chain and calls AWS APIs directly.

## Current status

shimesu is a v0.x, single-operator MVP. The CLI is intended for one operator or trusted AWS environment, and breaking changes may occur before a 1.0 release. The implemented commands are:

- `shimesu status`
- `shimesu stack init --domain <domain>`
- `shimesu stack status`
- `shimesu stack update`
- `shimesu stack destroy --confirm`
- `shimesu stack teardown --confirm-data-loss`
- `shimesu doctor`
- `shimesu publish <path>`
- `shimesu site list`
- `shimesu site inspect <slug>`
- `shimesu site delete <slug>`

Earlier pre-release command names are not retained as aliases. A first publish creates the DynamoDB site record and later publishes update it. There is no separate `site create` command and no deployment-history command. Traffic analytics, rollback automation, custom per-site domains, dynamic application hosting, and a web dashboard are not implemented.

## Architecture

An installation uses two CloudFormation stacks:

```text
us-east-1
└── shimesu-certificate
    └── ACM certificate for the base domain and wildcard

eu-central-1 by default
└── shimesu
    ├── private, encrypted, versioned S3 bucket
    ├── shared CloudFront distribution with OAC
    ├── CloudFront Function for host-to-prefix routing
    └── DynamoDB site table using PAY_PER_REQUEST
```

CloudFront requires its ACM certificate in `us-east-1`. Stateful content and metadata live in the selected regional stack, which defaults to `eu-central-1`.

The CloudFront Function maps site hosts to private S3 prefixes:

- `report.static.example.com` -> `report/index.html`
- `report.static.example.com/guide/` -> `report/guide/index.html`

Requests ending in `/` resolve to the `index.html` inside that directory, so multi-page sites with pretty URLs work without extra configuration.

The S3 bucket is not public. CloudFront can read it only through Origin Access Control. DynamoDB is the sole authoritative site metadata store.

## Prerequisites

- An AWS account and credentials available through the normal AWS provider chain
- A base domain such as `static.example.com` (normalized to lowercase automatically)
- Control of either a matching Route 53 hosted zone or an external DNS provider
- Rust 1.94.1 or newer


## Install the CLI

Install the published CLI from crates.io:

```bash
cargo install shimesu
shimesu --version
```

For development or template-based manual provisioning, use a source checkout:

```bash
git clone https://github.com/michidk/shimesu.git
cd shimesu
cargo install --path cli
```

## Provision an installation

One command creates or resumes the certificate stack in `us-east-1` (required by CloudFront — this is an AWS constraint, not a choice), waits for certificate issuance, and then creates the data plane in your chosen region:

```bash
shimesu stack init --domain static.example.com
```

The managed certificate stack is named `<stack-name>-certificate`. Repeated invocations reuse it when its `BaseDomain` matches. If certificate validation is still in progress, rerunning the command resumes the same stack instead of requesting another certificate.

Current installations tag both CloudFormation stacks and the managed ACM certificate with `Application=shimesu` and `StackName=<stack-name>`. Update, destroy, and teardown refuse resources without those exact tags. Installations created by an older pre-tag build must be tagged manually or recreated before the current CLI will mutate them; this conservative migration rule prevents a same-named unrelated stack from being changed or deleted.

For external DNS, the CLI writes the exact ACM validation CNAME to stderr. Add that record at the DNS provider, leave it in place for renewal, and rerun the same command if the first invocation times out while waiting for validation.

For a Route 53 hosted zone, pass its ID so both certificate validation and the final CloudFront records are managed automatically:

```bash
shimesu stack init \
  --domain static.example.com \
  --hosted-zone-id Z1234567890EXAMPLE
```

An existing `us-east-1` certificate covering the base domain and wildcard can be supplied as an advanced override. In that mode the CLI does not create or inspect a certificate stack:

```bash
shimesu stack init \
  --domain static.example.com \
  --certificate-arn arn:aws:acm:us-east-1:123456789012:certificate/EXAMPLE
```

Omit `--hosted-zone-id` for externally managed DNS. After the stack is created, the CLI prints the exact CNAME records for the base domain and wildcard subdomains. JSON output includes `distribution_domain_name` and a `dns_records` array with each record's `name`, `type`, and `value`.

### Verify the installation

After `shimesu stack init` completes and DNS has propagated, open the base domain in a browser:

```text
https://static.example.com
```

Shimesu installs a simple verification page there automatically. Seeing it confirms that DNS, TLS, CloudFront routing, and access to the private S3 origin are working together. If it does not appear, run `shimesu doctor` and `shimesu stack status` to identify the failing layer.

After upgrading the CLI, apply infrastructure changes with:

```bash
shimesu stack update
```

## Configure the CLI

The optional configuration file is `~/.config/shimesu/config.toml`:

```toml
[installation]
stack_name = "shimesu"
region = "eu-central-1"
# profile = "my-profile"
```

Precedence is CLI flag, environment variable, config file, then hardcoded default.

| Setting | Flag | Environment variable | Default |
| --- | --- | --- | --- |
| Stack | `--stack` | `SHIMESU_STACK` | `shimesu` |
| Region | `--region` | `AWS_REGION`, then `AWS_DEFAULT_REGION` | `eu-central-1` |
| Profile | `--profile` | `AWS_PROFILE` | AWS provider-chain default |

The CLI never stores access keys or session tokens.

## IAM permissions by command

The CLI uses a small set of direct AWS API calls. The required IAM actions per command are listed below.

| Command | Direct IAM actions used by the CLI | Notes |
| --- | --- | --- |
| `shimesu status` | `sts:GetCallerIdentity`, `cloudformation:DescribeStacks` | Reads caller identity and regional stack outputs. |
| `shimesu doctor` | `sts:GetCallerIdentity`, `cloudformation:DescribeStacks`, `s3:HeadBucket`, `dynamodb:DescribeTable` | Checks credentials, stack outputs, bucket reachability, and DynamoDB availability. |
| `shimesu stack init` | `cloudformation:CreateStack`, `cloudformation:DescribeStacks`, `cloudformation:DescribeStackEvents`, `cloudformation:ListStackResources`, `acm:DescribeCertificate`, optional `route53:GetHostedZone`, `s3:PutObject` | The CLI creates or resumes the certificate stack in `us-east-1`, discovers its ACM resource, validates certificate status and coverage, and uploads the bundled installation assets after the regional stack finishes. `GetHostedZone` is used only with `--hosted-zone-id` to prove that the selected zone owns the requested domain. CloudFormation also needs permission to create the S3, CloudFront, DynamoDB, ACM, and optional Route 53 resources declared in the templates. |
| `shimesu stack status` | `cloudformation:DescribeStacks` | Reads stack status and outputs only. |
| `shimesu stack update` | `cloudformation:UpdateStack`, `cloudformation:DescribeStacks`, `cloudformation:DescribeStackEvents`, `s3:PutObject` | Reuses prior stack parameters and applies the current template and bundled installation assets. |
| `shimesu stack destroy --confirm` | `cloudformation:DeleteStack`, `cloudformation:DescribeStacks`, `cloudformation:DescribeStackEvents`, `acm:ListTagsForCertificate` | Deletes only the regional stack after verifying Shimesu ownership. The `us-east-1` certificate probe verifies managed ACM tags for retention reporting; probe failures are reported as unknown and never block regional deletion. |
| `shimesu stack teardown --confirm-data-loss` | `cloudformation:DeleteStack`, `cloudformation:DescribeStacks`, `cloudformation:DescribeStackEvents`, `s3:ListAllMyBuckets`, `s3:GetBucketTagging`, `s3:ListBucketVersions`, `s3:DeleteObject`, `s3:DeleteObjectVersion`, `s3:DeleteBucket`, `dynamodb:DescribeTable`, `dynamodb:ListTagsOfResource`, `dynamodb:DeleteTable`, `acm:ListTagsForCertificate`, `acm:DeleteCertificate` | Permanently deletes the regional stack, all retained installation-owned S3 buckets and their versioned objects and delete markers, the DynamoDB table, and only a certificate stack and ACM certificate carrying exact installation ownership tags. Operator-supplied external certificates from `--certificate-arn` are never deleted. Resumable if the regional stack was already destroyed. |
| `shimesu publish <path>` | `s3:ListBucket`, `s3:GetObject`, `s3:PutObject`, `s3:DeleteObject`, `dynamodb:GetItem`, `dynamodb:UpdateItem`, `cloudfront:CreateInvalidation` | Compares existing objects, uploads changed files, prunes stale files below `{slug}/`, records deployment metadata, and invalidates `/{slug}/*`. |
| `shimesu site list` | `cloudformation:DescribeStacks`, `dynamodb:Scan` | Reads the table name from stack outputs, then scans site metadata. |
| `shimesu site inspect <slug>` | `cloudformation:DescribeStacks`, `dynamodb:GetItem` | Reads one site record and reconstructs the canonical URL from the base domain output. |
| `shimesu site delete <slug>` | `cloudformation:DescribeStacks`, `dynamodb:DeleteItem`, `s3:ListBucket`, `s3:DeleteObject`, `cloudfront:CreateInvalidation` | `CreateInvalidation` is skipped when `--keep-files` is used. |

If you use the raw AWS CLI flows in this README instead of the built-in `shimesu stack init` and `shimesu stack update` commands, add the CloudFormation change-set and template-inspection permissions used by `aws cloudformation deploy`: `cloudformation:CreateChangeSet`, `cloudformation:DescribeChangeSet`, `cloudformation:ExecuteChangeSet`, `cloudformation:DeleteChangeSet`, `cloudformation:ListStackResources`, `cloudformation:GetTemplateSummary`, and `cloudformation:ValidateTemplate`.

## Publish content

Publish one HTML file as a site's root document:

```bash
shimesu publish ./report.html --yes
```

This derives the slug `report` and uploads the file as `report/index.html`.

Publish a directory while preserving its safe relative tree:

```bash
shimesu publish ./dist --site docs --yes
```

Publish a zip archive after bounded zip-slip and size validation:

```bash
shimesu publish ./site.zip --site docs --yes
```

Every deployment kind is limited to 500 MB and 10,000 files. Content is hashed up front and streamed from disk during upload with bounded concurrency, so large deployments do not need to fit in memory. Publishing validates the site slug and every local path, rejects unsafe symlinks and archive entries, computes SHA-256 metadata, skips unchanged objects, uploads changed objects with detected content types, and prunes stale objects only below `{slug}/`. Uploaded objects receive default `Cache-Control` headers: `public, max-age=60` for HTML and `public, max-age=86400` for other assets, so browsers refresh pages quickly while assets stay cached. After synchronization succeeds, the CLI updates the DynamoDB site record and creates a `/{slug}/*` CloudFront invalidation.

Deployments are idempotent but not transactionally atomic across S3, DynamoDB, and CloudFront. If an operation is interrupted, rerun the same publish. The site record stores only the latest deployment identifier and aggregate counts; S3 versioning provides manual recovery but there is no rollback command.

## Inspect and remove sites

```bash
shimesu status
shimesu stack status
shimesu doctor
shimesu site list
shimesu site inspect docs
shimesu site delete docs --yes
```

Use `--keep-files` to remove only the DynamoDB record:

```bash
shimesu site delete docs --keep-files --yes
```

Deletion is bounded to the exact `{slug}/` prefix. After removing the files and the metadata record, the CLI creates a `/{slug}/*` CloudFront invalidation so the deleted site stops being served from edge caches immediately; the JSON output reports the `invalidation_id`. With `--keep-files` no invalidation is created. Because the bucket is versioned, deleting visible objects creates delete markers and older versions can continue to consume storage until the operator explicitly removes them.

## Automation and JSON output

All current commands accept `--json`, and redirected non-TTY stdout also selects JSON automatically. In JSON mode, successful results are written to stdout, while structured errors and human diagnostics are written to stderr. In human mode, status and progress lines use stdout and warnings/errors use stderr. Destructive or overwrite operations require `--yes` outside an interactive terminal.

```bash
shimesu status --json
shimesu stack status --json
shimesu doctor --json
shimesu site list --json
shimesu publish ./dist --site docs --yes --json
shimesu site inspect docs --json
shimesu site delete docs --yes --json
```

Stored timestamps and JSON timestamps use UTC and ISO 8601. JSON field meanings are treated as a backward-compatible public interface.

## AWS resources and cost

| Resource | Purpose | Retention | Primary cost drivers |
| --- | --- | --- | --- |
| S3 bucket | Private site files under per-site prefixes | Retained | Stored bytes, requests, versions |
| CloudFront distribution | Shared HTTPS delivery for the base and wildcard domains | Deleted with stack | Requests and data transfer |
| CloudFront Function | Host-to-prefix request rewrite | Deleted with stack | Function invocations |
| DynamoDB table | Authoritative site records, `PAY_PER_REQUEST` | Retained | Read and write requests, storage |
| ACM certificate | Base and wildcard TLS certificate in `us-east-1` | Retained | No direct certificate charge |
| Route 53 records | Optional DNS automation | Deleted with stack | Hosted-zone and DNS-query charges |

There are no NAT gateways, compute instances, containers, provisioned databases, or always-on workers. Idle cost is therefore near zero, but it is not guaranteed to be exactly zero. S3 versions, CloudFront requests and transfer, DynamoDB use, and optional Route 53 remain billable.

### Cost estimate

All figures are approximate US East pricing as of 2025. Your region and usage will vary.

Route 53 DNS automation is optional. If you manage DNS externally, shimesu prints the two CNAME records to create and requires no Route 53 access at all.

| Scenario | Estimated monthly cost | What drives it |
| --- | --- | --- |
| **Idle** — stack deployed, no visitors, external DNS | ~$0 | Nothing runs at rest; all billing follows actual usage |
| **Idle** — stack deployed, no visitors, Route 53 DNS | ~$0.50 | Route 53 hosted zone fee ($0.50/zone) |
| **Light** — a few sites, hundreds of requests/day, ~1 GB transfer | ~$1–3 | CloudFront transfer ($0.085/GB), requests, optional Route 53 queries |
| **Active** — multiple sites, thousands of requests/day, ~10 GB transfer | ~$5–15 | CloudFront transfer and request volume dominate |

Key line items:

- **Route 53** — $0.50/hosted zone/month + $0.40/million DNS queries; only applies when `--hosted-zone-id` is used
- **CloudFront data transfer** — $0.085/GB (first 10 TB/month); requests $0.0075/10,000 HTTPS requests
- **CloudFront Function** — $0.10/million invocations (one invocation per CDN request)
- **S3 storage** — $0.023/GB/month; GET requests from CloudFront $0.0004/1,000
- **DynamoDB** — effectively zero for typical site-metadata reads and writes at PAY_PER_REQUEST rates
- **ACM certificate** — free

New AWS accounts receive a CloudFront free tier for the first 12 months (1 TB transfer and 10 million requests/month). There is no free tier for Route 53 hosted zones.

## Security and data model

- S3 public-access blocking is enabled in all four modes.
- Bucket reads are limited to the installation's CloudFront distribution through OAC.
- CloudFront redirects HTTP to HTTPS and requires TLS 1.2 or newer.
- Uploaded HTML and JavaScript are treated as opaque bytes and never executed by the CLI.
- Site slugs are DNS-safe labels; `_shimesu` and other reserved names are rejected.
- Path traversal, absolute archive paths, control characters, unsafe symlinks, zip bombs, and bucket-wide site deletion are rejected.
- The CLI uses the AWS SDK retry behavior and standard credential chain. It does not log credentials or signed URLs.

Published site content is untrusted and shares the installation's parent domain. Do not set parent-domain cookies that should be hidden from site subdomains.

## Recovery without shimesu

All persistent data remains accessible with standard AWS tools:

```bash
aws dynamodb scan \
  --table-name shimesu-projects \
  --region eu-central-1

aws s3 sync \
  s3://YOUR_BUCKET/docs/ \
  ./recovered-docs/ \
  --region eu-central-1
```

Read the actual bucket and table names from the regional stack outputs instead of assuming generated names.

## Teardown

CloudFormation intentionally retains the S3 bucket, DynamoDB table, and managed ACM certificate. Ordinary `stack destroy` deletes only the regional stack; its output reports whether the data plane was deleted and whether a managed certificate stack remains. Use the explicit full-teardown command to remove installation-owned retained resources.

### One-command full teardown

```bash
shimesu stack teardown --confirm-data-loss
```

This performs a permanent, irreversible teardown of the entire installation in a single command:

1. Captures regional stack outputs if the stack still exists.
2. Deletes the regional CloudFormation stack and waits for completion.
3. Discovers and deletes every installation-owned retained S3 bucket, purging all object versions and delete markers in batches before removing the bucket. Ownership is verified by both the deterministic name prefix (`<stack>-contentbucket-`) and the `Application=shimesu` and `StackName=<stack>` tags. An expected bucket that fails the tag check causes an error rather than silent deletion.
4. Deletes the installation-owned DynamoDB table after confirming its ownership tags.
5. When the regional stack supplied independent base-domain proof, deletes the matching exact-tagged `<stack>-certificate` CloudFormation stack in `us-east-1` and waits for completion.
6. Deletes the retained ACM certificate only after the regional domain, certificate-stack tags, and certificate tags all identify the selected Shimesu installation. A certificate supplied through `--certificate-arn` remains operator-owned and is never deleted.

The command is resumable for retained S3 and DynamoDB data: if the regional stack was already deleted by an earlier `stack destroy --confirm`, teardown discovers those resources from deterministic names and exact ownership tags. Without the regional stack's independent domain output, certificate-stack and ACM deletion is skipped and must be completed manually. Older certificate stacks created before ownership tags were introduced are also deliberately refused.

Remove externally managed DNS records after teardown if CloudFormation did not own them.

### Manual step-by-step teardown

If you prefer to perform each step manually:

1. Back up any content and metadata you need.
2. Delete the regional stack: `shimesu stack destroy --confirm`. Global `--yes` does not replace the required `--confirm` flag.
3. Explicitly remove all versions and delete markers from the retained S3 bucket, then delete the bucket.
4. Explicitly delete the retained DynamoDB table.
5. Delete the managed certificate stack in `us-east-1`. Do not delete a same-named stack unless its ownership tags match the installation.
6. Explicitly delete the retained managed ACM certificate after CloudFront no longer uses it. Never delete a certificate supplied through `--certificate-arn` as part of installation cleanup.
7. Remove externally managed DNS records if CloudFormation did not own them.

`stack destroy` removes the regional CloudFormation stack but does not remove retained data or certificate resources. A managed `<stack>-certificate` stack and ACM certificate remain eligible for verified teardown; an override certificate remains operator-owned. Permanent cleanup remains deliberately manual and explicit unless `stack teardown --confirm-data-loss` is used.

## AI agent usage

A `SKILL.md` is included in the repository root. It teaches AI agents (Claude, GPT, and compatible assistants running [opencode](https://opencode.ai) or a compatible agent framework) how to use the full shimesu CLI — publishing, site management, stack operations, and diagnostics.

Install the skill once per machine:

```bash
mkdir -p ~/.agents/skills/shimesu
cp SKILL.md ~/.agents/skills/shimesu/SKILL.md
```

After installation, load it in any agent session with `/shimesu` or by asking your agent to publish a file. The skill covers the complete command surface, JSON output shapes, slug derivation rules, confirmation flags, and common error recovery.

## Development

The default test suite does not require an AWS account:

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cargo run -- --help
```

See [SECURITY.md](https://github.com/michidk/shimesu/blob/main/SECURITY.md) for the security model and how to report vulnerabilities.

[MIT License](https://github.com/michidk/shimesu/blob/main/LICENSE)
