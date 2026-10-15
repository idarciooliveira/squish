FROM --platform=linux/amd64 rust:1.99-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools nasm \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /app
COPY . .
RUN cargo build --release -p squish --target x86_64-unknown-linux-musl

FROM --platform=linux/amd64 scratch
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/squish /squish
USER 65532:65532
EXPOSE 8080
STOPSIGNAL SIGINT
ENTRYPOINT ["/squish", "serve", "--host", "0.0.0.0"]
