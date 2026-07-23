---
name: shimesu
description: >
  Use the shimesu CLI to publish static files to a self-hosted AWS installation
  and manage sites, stacks, and deployments.

  TRIGGER THIS SKILL WHEN:
  - User wants to publish a file, directory, or zip archive to a shimesu site
  - User wants to list, inspect, or delete a shimesu site
  - User asks about the canonical URL of a published site
  - User wants to check stack health, run doctor, or diagnose a shimesu installation
  - User wants to provision, update, destroy, or tear down a shimesu stack
  - User references /publish, /shimesu, or shimesu CLI commands

  SYMPTOMS that indicate this skill is needed:
  - Agent calls S3 or CloudFront APIs directly instead of using the CLI
  - Agent tries to run `shimesu deploy` (wrong — it's `shimesu publish`)
  - Agent omits --yes in non-interactive mode and hangs on a prompt
  - Agent uses --confirm on publish instead of --yes
metadata:
  version: 1
---

# Shimesu CLI Skill

## Configuration

Optional config file: `~/.config/shimesu/config.toml`

```toml
[installation]
stack_name = "shimesu"    # also: env var SHIMESU_STACK
region     = "eu-central-1"  # set explicitly; omit to let AWS SDK resolve from environment
# profile  = "my-profile"
```

Precedence: **CLI flag > config file > AWS SDK environment resolution**.

| Setting | Flag | Env var | Fallback |
|---|---|---|---|
| Stack name | `--stack` | `SHIMESU_STACK` | `shimesu` |
| Region | `--region` | — | AWS SDK chain (`AWS_REGION`, profile, etc.) |
| Profile | `--profile` | `AWS_PROFILE` | AWS provider-chain default |

## Global flags

Apply to every command:

```
--profile <NAME>    AWS profile
--region  <REGION>  AWS region
--stack   <NAME>    Stack name
--json              Result → stdout, diagnostics → stderr
--yes               Skip confirmation prompts
```

---

## Commands

### `shimesu publish <path>`

First publish creates the site; later publishes replace all content in that site's prefix (stale files pruned). Prompts before replacing an existing site unless `--yes`.

```
shimesu publish <PATH> [-s <SLUG>] [--yes] [--json]
```

**Slug derivation** (filename stem or dirname when `--site` is omitted):

| Input | Slug |
|---|---|
| `./report.html` | `report` |
| `./dist/` | `dist` |
| `./slides.zip` | `slides` |
| `--site my-site` | `my-site` |

Slugs are normalized to DNS labels. `_shimesu` prefix is reserved. Limits: 500 MB, 10,000 files.

```bash
shimesu publish ./report.html --yes
shimesu publish ./dist --site docs --yes --json
```

**JSON output:**

```json
{
  "slug": "docs",
  "url": "https://docs.static.example.com",
  "deployment_id": "01J...",
  "file_count": 12,
  "total_bytes": 204800,
  "uploaded": 5,
  "skipped": 7,
  "deleted": 1,
  "updated_at": "2026-07-23T10:00:00Z",
  "invalidation_id": "ABCDE12345"
}
```

CloudFront invalidation is async (~60 s). New subdomains may take a few minutes for DNS on first publish.

---

### `shimesu site list`

```bash
shimesu site list [--json]
```

### `shimesu site inspect <slug>`

```bash
shimesu site inspect docs [--json]
```

### `shimesu site delete <slug>`

Removes metadata and S3 files (bounded to `<slug>/` only), then invalidates the CDN. `--keep-files` skips file deletion and invalidation.

```bash
shimesu site delete docs --yes [--keep-files] [--json]
```

---

### `shimesu status` / `shimesu stack status`

Print stack outputs (base domain, distribution, bucket, table).

```bash
shimesu status [--json]
```

### `shimesu doctor`

Verify credentials, stack outputs, S3, and DynamoDB. Run first when something is broken.

```bash
shimesu doctor [--json]
```

---

### `shimesu stack init`

Provision a new installation (certificate stack in `us-east-1` + regional data plane).

```bash
shimesu stack init --domain static.example.com [--hosted-zone-id Z...] [--certificate-arn arn:...]
```

Repeated invocations resume an in-progress stack.

### `shimesu stack update`

Apply the current template to an existing stack. Run after upgrading the CLI.

```bash
shimesu stack update
```

### `shimesu stack destroy --confirm`

Delete the regional stack. Retains S3 bucket, DynamoDB table, and ACM certificate. `--confirm` is mandatory; `--yes` does not replace it.

```bash
shimesu stack destroy --confirm
```

### `shimesu stack teardown --confirm-data-loss`

Permanently destroy everything: stack, retained bucket (all versions), DynamoDB table, certificate stack, managed ACM certificate. Irreversible. Resumable if the regional stack was already deleted.

```bash
shimesu stack teardown --confirm-data-loss
```

---

## Common errors

| Error | Cause | Fix |
|---|---|---|
| `Stack outputs not found` | Stack missing or wrong name | `shimesu stack status`; check `--stack` / `SHIMESU_STACK` |
| `Cannot prompt... Use --yes` | Non-interactive without `--yes` | Add `--yes` |
| `slug is reserved` | Slug starts with `_shimesu` | Choose a different name |
| `Cannot access deployment path` | Path missing or is a symlink | Verify path; resolve symlinks first |
| `exceeds size limit` | > 500 MB or > 10,000 files | Reduce content or split |
| `Stack in ROLLBACK_COMPLETE` | Previous init failed | Delete the failed stack, re-run `stack init` |
| 404 after publish | Invalidation or DNS not propagated | Wait ~60 s; new subdomains need a few minutes |

## AWS CLI fallback

If `shimesu` is unavailable or a command fails unrecoverably, use the AWS CLI directly. First resolve the installation's stack outputs to get the actual resource names:

```bash
aws cloudformation describe-stacks \
  --stack-name shimesu \
  --region eu-central-1 \
  --query 'Stacks[0].Outputs'
```

Key output keys: `BucketName`, `TableName`, `BaseDomain`, `DistributionId`.

**Check stack health**
```bash
aws cloudformation describe-stacks --stack-name shimesu --region eu-central-1 \
  --query 'Stacks[0].StackStatus'
```

**List sites**
```bash
aws dynamodb scan --table-name shimesu-projects --region eu-central-1
```

**Publish a single file** (slug = `report`, file becomes `report/index.html`)
```bash
aws s3 cp ./report.html s3://BUCKET/report/index.html \
  --content-type text/html \
  --cache-control "public, max-age=60"
```

**Publish a directory**
```bash
aws s3 sync ./dist/ s3://BUCKET/docs/ \
  --delete \
  --cache-control "public, max-age=86400"
```

**Invalidate CDN after manual publish**
```bash
aws cloudfront create-invalidation \
  --distribution-id DISTRIBUTION_ID \
  --paths "/docs/*"
```

**Recover site files**
```bash
aws s3 sync s3://BUCKET/docs/ ./recovered-docs/ --region eu-central-1
```

Manual publishes bypass slug validation, DynamoDB metadata updates, hash-based diffing, and content-type detection — use `shimesu publish` whenever possible.

## Safety invariants

- Only delete or prune files within `<slug>/` — never bucket-wide.
- `--confirm` (destroy) and `--confirm-data-loss` (teardown) are mandatory; `--yes` does not replace them.
- Never delete a certificate from `--certificate-arn` during teardown.
- Confirm before replacing an existing site unless `--yes` is set.
