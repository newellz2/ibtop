FROM ubuntu:24.04

ARG DOCA_PACKAGES=""
ARG DOCA_VERSION=3.1.0

ARG DOCA_PREPUBLISH=false
ARG DOCA_DISTRO="ubuntu24.04"
ARG DOCA_ARCH="arm64-sbsa"

# Avoid prompts from apt
ENV DEBIAN_FRONTEND=noninteractive

# Install basic dependencies
RUN apt-get update && apt-get install -y \
    curl \
    gnupg \
    ca-certificates \
    build-essential \
    pkg-config \
    git \
    clang \
    && rm -rf /var/lib/apt/lists/*

RUN echo "Installing DOCA ${DOCA_VERSION}..." && \
    # Determine base URL
    if [ "${DOCA_PREPUBLISH}" = "true" ]; then \
        BASE_URL="https://doca-repo-prod.nvidia.com/public/repo/doca"; \
    else \
        BASE_URL="https://linux.mellanox.com/public/repo/doca"; \
    fi && \
    DOCA_URL="${BASE_URL}/${DOCA_VERSION}/${DOCA_DISTRO}/${DOCA_ARCH}/" && \
    echo "Using DOCA URL: ${DOCA_URL}" && \
    curl -fsSL ${BASE_URL}/GPG-KEY-Mellanox.pub | gpg --dearmor > /etc/apt/trusted.gpg.d/GPG-KEY-Mellanox.pub && \
    echo "deb [signed-by=/etc/apt/trusted.gpg.d/GPG-KEY-Mellanox.pub] ${DOCA_URL} ./" > /etc/apt/sources.list.d/doca.list && \
    cat /etc/apt/sources.list.d/doca.list && \
    apt-get update && \
    apt-get upgrade -y && \
    rm -rf /var/lib/apt/lists/*


RUN apt-get update; \
    apt-get install -y ${DOCA_PACKAGES} \
    libibumad-dev \
    libibnetdisc-dev \
    libibmad-dev  && \
    rm -rf /var/lib/apt/lists/*

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# Set working directory
WORKDIR /build

# Copy both rsmad and ibtop
COPY rsmad/ /build/rsmad/
COPY ibtop/ /build/ibtop/

# Build the project
WORKDIR /build/ibtop
RUN cargo build --release

RUN ls -la /build/ibtop/target/release/; \
    cp /build/ibtop/target/release/ibtop /usr/local/bin/ibtop

CMD ["/build/ibtop/target/release/ibtop"]

