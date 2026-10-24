# Project maintenance

- Keep the console independent from routing, DNS and subscription service lifecycles.
- Keep plugins optional; separate enablement, visibility and service health.
- Keep collectors bounded, cached and cancellable. Do not spawn shell commands for high-frequency metrics.
- Use explicit, authorized actions. Do not expose arbitrary commands or arbitrary-target proxies.
- Never commit credentials, subscription URLs, live profiles or deployment-specific data.
- Keep runtime overhead small. Frontend build tooling must not become a console runtime dependency.
- Distinguish design targets, measured performance, successful builds and live verification.
- Update README and architecture documentation when public interfaces or supported commands change.
