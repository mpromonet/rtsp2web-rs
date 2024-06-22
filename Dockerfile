FROM rust as builder
LABEL maintainer=michel.promonet@free.fr

WORKDIR /workdir

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release

COPY ./src ./src
RUN cargo build --release

FROM rust
WORKDIR /app

COPY --from=builder /workdir/target/release/rtsp2web-rs .

ENTRYPOINT ["./rtsp2web-rs"]