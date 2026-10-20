# Admin API shares the public host and port, protected only by Admin Tokens

The admin API is served under `/admin/...` on the same host and port as the voter UI and voter API, instead of on a separate admin port. Authentication is an Admin Token sent as a bearer token. Admin Tokens are created, listed and revoked only through a local CLI on the server, carry an optional expiry, and are stored as hashes in the database. We chose this so that one listener, one reverse proxy and one TLS certificate are enough to run the app, while access can still be handed out and withdrawn without restarts.

## Consequences

- Network isolation is no longer a second layer of protection: Admin Tokens alone guard every Voter Reference and Response, so they must be long and random, shown once, and kept out of logs.
- Admin routes must sit in their own router behind the token check, so a routing mistake can't expose them without authentication.
- There is no way to create Admin Tokens over HTTP; whoever can run the CLI against the database is the root of trust.
- Moving admin to a separate port later is still possible but changes every admin client's URL.
