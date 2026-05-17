[![FOSSA Status](https://app.fossa.com/api/projects/git%2Bgithub.com%2Fmpromonet%2Frtsp2web-rs.svg?type=shield)](https://app.fossa.com/projects/git%2Bgithub.com%2Fmpromonet%2Frtsp2web-rs?ref=badge_shield)

rtsp2web-rs
===

try to write in rust https://github.com/mpromonet/rtsp2web

```
cargo run -- -C config.json -k key.pem -c cert.pem
```

With QUIC / WebTransport support (add `-q <port>`, requires TLS):
```
cargo run -- -C config.json -k key.pem -c cert.pem -q 4433
```

Build & Run app
```
cargo build
target/debug/rtsp2web-rs -C config.json -k key.pem -c cert.pem -q 4433
```

Run docker image
```
docker run -p 8080:8080 ghcr.io/mpromonet/rtsp2web-rs:latest
```


## License
[![FOSSA Status](https://app.fossa.com/api/projects/git%2Bgithub.com%2Fmpromonet%2Frtsp2web-rs.svg?type=large)](https://app.fossa.com/projects/git%2Bgithub.com%2Fmpromonet%2Frtsp2web-rs?ref=badge_large)