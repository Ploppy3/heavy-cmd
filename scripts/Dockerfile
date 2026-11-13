# The toolchain every check, build and release runs in: Rust, mingw-w64 to build the Windows
# binary, musl to build a static Linux one, Wine to run the Windows tests. scripts/dev builds it.
FROM rust:1.99.1-slim-trixie

RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 musl-tools wine wine64 git curl ca-certificates zip unzip \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add x86_64-unknown-linux-musl x86_64-pc-windows-gnu \
 && rustup component add clippy rustfmt

# Wine is how a Windows test binary runs here.
ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUNNER=wine \
    WINEDEBUG=-all \
    WINEDLLOVERRIDES="mscoree,mshtml="
