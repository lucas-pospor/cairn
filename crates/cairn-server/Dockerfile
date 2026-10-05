# Build:  docker build -f crates/cairn-server/Dockerfile -t cairn-server .
# (run from the repository root)

FROM rust:1-bookworm AS build
WORKDIR /src
# Only the crates the server needs.
COPY Cargo.toml Cargo.lock ./
COPY crates/cairn-core crates/cairn-core
COPY crates/cairn-sync crates/cairn-sync
COPY crates/cairn-server crates/cairn-server
# The workspace lists the app too; drop it so the build does not need it.
RUN sed -i 's|, "app/src-tauri"||' Cargo.toml \
 && cargo build --release -p cairn-server

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home /data cairn \
 && mkdir -p /data && chown cairn /data
COPY --from=build /src/target/release/cairn-server /usr/local/bin/cairn-server
USER cairn
ENV CAIRN_DATA=/data CAIRN_ADDR=0.0.0.0:8787
VOLUME /data
EXPOSE 8787
HEALTHCHECK --interval=30s --timeout=3s CMD ["/usr/local/bin/cairn-server", "--health"]
CMD ["/usr/local/bin/cairn-server"]
