FROM docker.io/library/rust:1.99.0-bookworm@sha256:114c7a4425406451c2866b6aafe69fe29b1b298832db1277d411ac73c82d04d6 AS rust_toolchain
FROM public.ecr.aws/amazonlinux/amazonlinux:2023@sha256:5c38fd85b00ce8c094315d9b71f671f880052314a22bd25f1a027821bf61d9d7

RUN dnf install -y gcc glibc-devel binutils git python3 && dnf clean all
COPY --from=rust_toolchain /usr/local/rustup /usr/local/rustup
COPY --from=rust_toolchain /usr/local/cargo/bin /usr/local/cargo/bin

ENV PATH="/usr/local/cargo/bin:${PATH}" \
    CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    RUSTUP_TOOLCHAIN=1.99.0 \
    CARGO_BUILD_JOBS=2 \
    LC_ALL=C
