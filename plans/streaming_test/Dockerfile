# Build Stage
FROM rust:1.57.0 AS builder
WORKDIR /usr/src/
RUN rustup target add x86_64-unknown-linux-musl

RUN USER=root cargo new rust_container
WORKDIR /usr/src/rust_container
COPY plan/Cargo.toml plan/Cargo.lock ./
RUN cargo build --release

COPY plan/src ./src
RUN cargo install --target x86_64-unknown-linux-musl --path .

# Bundle Stage
FROM scratch
COPY --from=builder /usr/local/cargo/bin/live_test .
# live_test is the crate name
EXPOSE 6060
ENTRYPOINT [ "/live_test"]