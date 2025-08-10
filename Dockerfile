# Multi-stage build for optimal container size
FROM rust:1.89-alpine AS builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    pkgconfig \
    fuse-dev \
    fuse-static \
    linux-headers

# Set working directory
WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY rhc/ ./rhc/
COPY metadata/ ./metadata/
COPY storage/ ./storage/
COPY client/ ./client/
COPY common/ ./common/
COPY proto/ ./proto/

# Build release binaries
RUN cargo build --release --bin lismount

# Runtime stage
FROM alpine:3.18

# Install runtime dependencies
RUN apk add --no-cache \
    fuse \
    fuse-dev \
    iproute2 \
    iputils \
    tcpdump \
    bash \
    curl \
    && rm -rf /var/cache/apk/*

# Create app directory
WORKDIR /app

# Copy built binaries
COPY --from=builder /app/target/release/lismount /app/target/release/lismount

# Create mount point
RUN mkdir -p /mnt/lis

# Setup FUSE
RUN echo 'user_allow_other' >> /etc/fuse.conf

# Set executable permissions
RUN chmod +x /app/target/release/lismount

# Default command
CMD ["/app/target/release/lismount", "lis-cluster:/", "/mnt/lis"]