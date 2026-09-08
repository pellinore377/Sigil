FROM rust:1.98-slim-bookworm@sha256:af0579d28b9a7ec5251aaafcb0c0a23dcde5c97065112aae0cc3abeda42d5394 AS toolchain
FROM toolchain AS source
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
FROM source AS build
RUN --mount=type=cache,target=/src/target cargo build --locked --release -p sigil-server && cp target/release/sigil-server /usr/local/bin/sigil-server

FROM eclipse-temurin:21-jdk-jammy@sha256:ce5767b7222312d42395f5bab033cd91f09e44032a2f21bdfd7b5b912dbe1e77 AS java
FROM toolchain AS web-tools
COPY --from=java /opt/java/openjdk /opt/java/openjdk
ENV JAVA_HOME=/opt/java/openjdk
ENV PATH="/opt/java/openjdk/bin:${PATH}"
RUN apt-get update && apt-get install -y --no-install-recommends curl unzip ca-certificates && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://services.gradle.org/distributions/gradle-8.13-bin.zip -o /tmp/gradle.zip && echo '20f1b1176237254a6fc204d8434196fa11a4cfb387567519c61556e8710aed78  /tmp/gradle.zip' | sha256sum -c - && unzip -q /tmp/gradle.zip -d /opt && rm /tmp/gradle.zip
RUN rustup target add wasm32-unknown-unknown && cargo install --locked wasm-bindgen-cli --version 0.2.127
FROM web-tools AS web-build
WORKDIR /src
COPY --from=source /src /src
RUN cargo build --locked --release --target wasm32-unknown-unknown -p sigil-core && wasm-bindgen target/wasm32-unknown-unknown/release/sigil_core.wasm --target web --out-dir target/web
COPY build.gradle.kts settings.gradle.kts gradle.properties ./
COPY app app
COPY shared shared
COPY kotlin-js-store kotlin-js-store
RUN /opt/gradle-8.13/bin/gradle --no-daemon :shared:wasmJsBrowserDistribution --console=plain

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867
RUN mkdir -p /var/lib/sigil && chown 65532:65532 /var/lib/sigil && chmod 700 /var/lib/sigil
COPY --from=build /usr/local/bin/sigil-server /usr/local/bin/sigil-server
COPY --from=web-build /src/shared/build/dist/wasmJs/productionExecutable /usr/share/sigil/web
COPY licenses/Server-ThirdParty.txt licenses/Crypto-ThirdParty.txt licenses/Client-ThirdParty.txt licenses/Text-ThirdParty.txt licenses/Integrations-ThirdParty.txt licenses/Calls-ThirdParty.txt /usr/share/doc/sigil/
COPY licenses/Web-ThirdParty.txt licenses/Newsreader.txt licenses/GoogleSansFlex.txt licenses/GoogleSansCode.txt licenses/GoogleSansCode-Trademarks.md licenses/MaterialSymbols.txt /usr/share/doc/sigil/
USER 65532:65532
ENV SIGIL_DATA_DIR=/var/lib/sigil/data SIGIL_LISTEN=0.0.0.0:8080
ENV SIGIL_WEB_DIR=/usr/share/sigil/web
EXPOSE 8080 34780/udp
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 CMD ["/usr/local/bin/sigil-server", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/sigil-server"]
