# Held offline Club Mode (`league-tui`) container build.
# Built and smoke-tested in CI (`--network=none`); not published to any registry yet.

FROM docker.io/library/rust:1.100.0-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS builder
WORKDIR /work

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY vendor ./vendor

# Only `xrpl-rust` is vendored under `vendor/`; fetch locked crates.io dependencies first,
# then build `league-tui` strictly offline (`CARGO_NET_OFFLINE=true --offline`) and stage
# an empty `/saves` directory owned by distroless `nonroot` (`65532:65532`).
RUN rustc --version && cargo --version && cargo fetch --locked
RUN CARGO_NET_OFFLINE=true cargo build --release --locked --offline -p league-tui --bin league-tui \
    && install -d -o 65532 -g 65532 -m 0755 /saves

FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

COPY --from=builder /work/target/release/league-tui /usr/local/bin/league-tui
COPY LICENSE /usr/share/doc/merchant-league/LICENSE
COPY --from=builder --chown=65532:65532 /saves /saves

ENV TERM=xterm-256color
USER 65532:65532
WORKDIR /saves
ENTRYPOINT ["/usr/local/bin/league-tui"]
