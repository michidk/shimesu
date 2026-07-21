# Security

## Supported versions

Only the current `main` branch is supported. There are no backport releases.

## Reporting a vulnerability

Please use [GitHub's private vulnerability reporting](https://github.com/michidk/shimesu/security/advisories/new) for this repository. Do not open a public issue for a security finding.

Include as much detail as you can: what the vulnerability is, how to reproduce it, and what impact you believe it has. We'll acknowledge receipt and work with you on a fix before any public disclosure.

There is no bug bounty program.

## Security model

shimesu is a self-hosted tool. The operator runs it with their own AWS credentials in their own account. The security properties below describe what the tool enforces by design.

**Content is private by default.** The S3 bucket has public-access blocking enabled in all four modes. CloudFront reads the bucket only through Origin Access Control. There is no public bucket policy.

**HTTPS only.** CloudFront redirects HTTP to HTTPS and requires TLS 1.2 or newer. The ACM certificate covers the base domain and the wildcard subdomain.

**Uploaded content is opaque.** The CLI treats HTML, JavaScript, and all other uploaded files as raw bytes. It does not parse, execute, or transform them.

**Scoped deletes.** Every delete and prune operation is bounded to the exact `{slug}/` prefix for the target site. Bucket-wide deletes are rejected. A deletion must prove its target is inside the correct installation and site namespace before issuing any AWS delete calls.

**Slug and path validation.** Site slugs must be valid DNS labels. Path traversal, absolute paths, control characters, unsafe symlinks, zip-slip entries, and zip bombs are all rejected at the boundary.

**No stored credentials.** The CLI uses the standard AWS credential provider chain and never writes access keys, session tokens, or signed URLs to disk or logs.

**Least-privilege IAM.** `policy.json` is the reference IAM policy for the operator. It is scoped to the actions and resource patterns used by the CLI and templates. Review and narrow its wildcard account, region, hosted-zone, and generated-resource patterns before use. It covers the default `shimesu` stack and names beginning with `shimesu-`; adjust the ARN patterns for other stack names.

**Shared parent domain.** All published sites share the installation's base domain. Published content is untrusted. Do not set parent-domain cookies that should be hidden from site subdomains, and do not host content that requires strict origin isolation on the same installation.

**Retained data.** The S3 bucket and DynamoDB table carry `DeletionPolicy: Retain`. Ordinary stack deletion does not destroy content. Permanent removal is explicit and manual.
