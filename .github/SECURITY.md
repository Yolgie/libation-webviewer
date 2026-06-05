# Security policy

Please report security issues privately via this repo's Security tab
→ **Report a vulnerability** (GitHub Security Advisories). Do not file
a public issue for security reports.

The repository enables Dependabot security alerts, secret scanning, and
push protection. CodeQL (Rust) runs on every PR and weekly. Container
images are scanned by Trivy on release; CRITICAL findings fail the
build. Images are signed via `cosign` (keyless OIDC, GitHub-issued
identity) and ship with a CycloneDX SBOM attestation.

## Verifying a release image

```
cosign verify ghcr.io/yolgie/libation-webviewer:<tag> \
  --certificate-identity-regexp 'https://github.com/yolgie/libation-webviewer/.github/workflows/release.yaml@.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```
