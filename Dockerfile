FROM rust AS builder
LABEL maintainer=michel.promonet@free.fr

WORKDIR /workdir

COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml

COPY ./src ./src
RUN cargo build --release

FROM rust:slim
WORKDIR /app

COPY --from=builder /workdir/target/release/rtsp2web-rs .

ENTRYPOINT ["./rtsp2web-rs"]