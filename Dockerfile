FROM rust:1.98-slim-bookworm@sha256:af0579d28b9a7ec5251aaafcb0c0a23dcde5c97065112aae0cc3abeda42d5394 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY vendor vendor
COPY core core
COPY crypto crypto
COPY client client
COPY android android
COPY protocol protocol
COPY text text
COPY media media
COPY maps maps
COPY calls calls
COPY server server
ENV RUSTFLAGS="--remap-path-prefix=/usr/local/cargo=/dependencies"
RUN cargo build --locked --release -p sigil-server

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867
RUN mkdir -p /var/lib/sigil && chown 65532:65532 /var/lib/sigil && chmod 700 /var/lib/sigil
COPY --from=build /src/target/release/sigil-server /usr/local/bin/sigil-server
COPY licenses/Server-ThirdParty.txt licenses/Crypto-ThirdParty.txt licenses/Client-ThirdParty.txt licenses/Text-ThirdParty.txt licenses/Integrations-ThirdParty.txt licenses/Calls-ThirdParty.txt /usr/share/doc/sigil/
USER 65532:65532
ENV SIGIL_DATA_DIR=/var/lib/sigil/data SIGIL_LISTEN=0.0.0.0:8080
EXPOSE 8080 34780/udp
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 CMD ["/usr/local/bin/sigil-server", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/sigil-server"]
