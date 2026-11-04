# Linux signing


Set `[linux_signing].identity_token_env` to an environment variable containing an OIDC identity token minted for the `sigstore` audience. Cargo-bundle uses Sigstore's production signing service; your CI identity provider must mint the token. Keep the token value out of the manifest.

```toml
[package.metadata.bundle.linux_signing]
identity_token_env = "SIGSTORE_ID_TOKEN"
```

With that variable present, packaging a `.deb` or `.AppImage` produces an adjacent detached JSON bundle, such as `Example.deb.sigstore.json`. The package bytes remain unchanged, and cargo-bundle reports the bundle path. This signs the package artifact, not an installed binary or repository metadata.

For CI, set `SIGSTORE_ID_TOKEN` from the provider's OIDC token step, then run `cargo bundle --format deb` or `cargo bundle --format appimage`. Verify the artifact and bundle with Cosign, using the expected OIDC issuer and signer identity:

```sh
cosign verify-blob \
  --bundle target/release/bundle/deb/example.deb.sigstore.json \
  --certificate-identity 'https://github.com/OWNER/REPOSITORY/.github/workflows/release.yml@refs/tags/v1.2.3' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' \
  target/release/bundle/deb/example.deb
```

Replace the sample identity, issuer, artifact path, and bundle path with the exact values for your OIDC provider and build. Sigstore's [blob signing guide](https://docs.sigstore.dev/cosign/signing/signing_with_blobs/) explains keyless identities and bundle files; its [verification guide](https://docs.sigstore.dev/cosign/verifying/verify/) documents `cosign verify-blob`.
