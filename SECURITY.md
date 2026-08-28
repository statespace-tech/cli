# Security policy

## Supported versions

Statespace is in alpha. The latest release receives security fixes.

## Report a vulnerability

Use GitHub private vulnerability reporting for the `statespace-tech/cli` repository. Do not open a public issue.

Include the affected component, impact, reproduction steps, and any suggested mitigation. Do not include real customer data.

We will confirm receipt, assess severity, and coordinate a fix and disclosure. We do not promise a response time during the alpha period.

## Sensitive data

Treat account sessions, admin tokens, and database tokens as secrets. A database URL identifies the account database, but a valid token is required for access.
