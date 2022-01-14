# Build Stage
FROM rust:1.58.0 AS builder

# Download the target for static linking.
# RUN rustup target add x86_64-unknown-linux-musl
# Required to compile ring lib.
# RUN apt update && apt install -y musl-tools
# Musl may have bad perf switch to debian buster slim if needed.

RUN USER=root cargo new live_test
WORKDIR /live_test

COPY plan/Cargo.toml plan/Cargo.lock ./
RUN cargo build --release

RUN rm src/*.rs

COPY plan/src ./src
# RUN cargo install --target x86_64-unknown-linux-musl --path .

RUN rm ./target/release/deps/live_test*
RUN cargo build --release

# Bundle Stage
FROM debian:buster-slim

# live_test is the crate name.
#COPY --from=builder /usr/local/cargo/bin/live_test .
COPY --from=builder /live_test/target/release/live_test .

EXPOSE 8080
ENTRYPOINT [ "./live_test"]