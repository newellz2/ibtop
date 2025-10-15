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

echo "Building ibtop Docker image with DOCA ${DOCA_VERSION}..."
echo "DOCA packages: ${DOCA_PACKAGES}"

if [ -d "../rsmad" ]; then
    echo "Found rsmad in parent directory, building from parent context..."
    cd ..
    docker build --debug \
        --build-arg DOCA_VERSION=${DOCA_VERSION} \
        --build-arg DOCA_PACKAGES="${DOCA_PACKAGES}" \
        -f ibtop/Dockerfile \
        -t ibtop:latest \
        -t ibtop:doca-${DOCA_VERSION} \
        .
    docker create --name ibtop-temp ibtop:latest
    docker cp ibtop-temp:/build/ibtop/target/release/ibtop ./ibtop
    # docker rm ibtop-temp
else
    echo "rsmad not found in parent directory, exiting..."
    exit 1
fi

echo ""
echo "Build complete!"
echo "  - ibtop:latest"
echo "  - ibtop:doca-${DOCA_VERSION}"
echo ""
echo "To run the container (replace mlx5_0 with your HCA device):"
echo "  docker run --rm -it --privileged --network host ibtop:latest --hca mlx5_0"
echo ""
echo "To run with a scope file:"
echo "  docker run --rm -it --privileged --network host ibtop:latest /build/ibtop/target/release/ibtop --hca mlx5_0 --scope-file /build/ibtop/examples/scope.csv"
echo ""
echo "For all available options:"
echo "  docker run --rm ibtop:latest --help"

