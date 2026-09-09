# Multi-stage build: compile rusno, then copy into a slim runtime image.
FROM rust:1.81 AS builder

WORKDIR /build
COPY . .
RUN cargo build --release --bin rusno

# --- Runtime ---
FROM debian:bookworm-slim

# Install runtime deps: docker CLI (for compose), git, ssh, ca-certificates
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        docker.io \
        docker-compose-v2 \
        git \
        openssh-client \
        ca-certificates \
        curl && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/rusno /usr/local/bin/rusno

# Default rusno home inside the container
ENV RUSNO_HOME=/root/.rusno

# Volumes:
#   /var/run/docker.sock  — host Docker socket (rw)
#   /root/.rusno          — rusno config, db, keys, ssh (rw)
#   /root/rusno/projects  — project git clones (rw)
#   /root/.ssh            — host SSH keys (ro, host-existing mode only)
VOLUME ["/var/run/docker.sock", "/root/.rusno", "/root/rusno/projects"]

EXPOSE 6967

ENTRYPOINT ["rusno", "serve"]
CMD ["--port", "6967"]
