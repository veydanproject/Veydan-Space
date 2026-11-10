This is `sqlx-sqlite` 0.9.0 from crates.io, with one change in `Cargo.toml`:
the `libsqlite3-sys` upper bound is `<0.40.0` instead of `<0.38.0`.

SQLx upstream made the same dependency change on its main branch. The local
patch lets all Veydan workspaces use `libsqlite3-sys` 0.39.1, including the
bundled SQLite 3.54.1, while keeping SQLx 0.9.0's API and code.
