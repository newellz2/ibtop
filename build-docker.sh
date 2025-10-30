#!/bin/bash

# Build script for ibtop Docker image
# This script handles the rsmad dependency correctly
#
# Usage:
#   ./build-docker.sh [DOCA_VERSION] [DOCA_PACKAGES]
#
# Examples:
#   ./build-docker.sh                    # Uses default version (3.1.0)
#   ./build-docker.sh 3.1.0              # Specify version
#   ./build-docker.sh 2.9.0              # Use older version
#   ./build-docker.sh 3.1.0 "doca-sdk"   # Specify packages to install

set -e

DOCA_VERSION=${1:-3.1.0}
DOCA_PACKAGES=${2:-""}
DOCA_ARCH=${3:-"arm64-sbsa"} #Other options: x86_64, arm64
UBUNTU_VERSION=${4:-"24.04"} #Other options: 20.04, 22.04

echo "Building ibtop Docker image with DOCA ${DOCA_VERSION}..."
echo "DOCA packages: ${DOCA_PACKAGES}"

if [ -d "../rsmad" ]; then
    echo "Found rsmad in parent directory, building from parent context..."
    cd ..
    docker build --debug \
        --build-arg DOCA_VERSION=${DOCA_VERSION} \
        --build-arg DOCA_PACKAGES="${DOCA_PACKAGES}" \
	--build-arg UBUNTU_VERSION="${UBUNTU_VERSION}" \
	--build-arg DOCA_ARCH="${DOCA_ARCH}" \
        -f ibtop/Dockerfile \
        -t ibtop:latest \
        .
    docker create --name ibtop-temp ibtop:latest
    docker cp ibtop-temp:/build/ibtop/target/release/ibtop ./ibtop
    docker container rm ibtop-temp
else
    echo "rsmad not found in parent directoy"
    echo "please clone it: git clone git@github.com/newellz2/rsmad"
    exit 1
fi

echo ""
echo "Build complete!"
echo "  - ibtop:latest"
