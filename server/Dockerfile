# One image, one binary. Build from the repository root:
#   docker build -f server/Dockerfile -t sigil-server .
FROM rust:1.94-bookworm AS build
WORKDIR /src
COPY protocol ./protocol
COPY server ./server
RUN cd server && cargo build --release --locked

FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/server/target/release/sigil-server /usr/local/bin/sigil-server
COPY --from=build /src/server/target/release/sigil-cli /usr/local/bin/sigil-cli
VOLUME ["/data"]
EXPOSE 8443
ENTRYPOINT ["/usr/local/bin/sigil-server"]
CMD ["--config", "/data/sigil.toml", "run"]
