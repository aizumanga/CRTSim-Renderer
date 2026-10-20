# CRTSim Renderer's render service, on Mesa's software Vulkan (lavapipe), so it needs no GPU,
# with FFmpeg for animations and videos. Build it from the repository's root:
#
#   docker build -f packaging/service/Dockerfile -t crtsim-service .
#   docker run --rm -p 8080:8080 -e CRTSIM_SERVICE_TOKEN=<16 or more characters> crtsim-service
#
# See docs/SERVICE.md.
FROM rust:1-trixie AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p crtsim-service \
    && python3 tools/licenses.py crtsim-service > dependency-licenses.txt

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends mesa-vulkan-drivers libvulkan1 ffmpeg \
    && rm -rf /var/lib/apt/lists/*
RUN useradd --system --uid 10001 --create-home crtsim
COPY --from=build /src/target/release/crtsim-service /usr/local/bin/crtsim-service
# Its own license and notices, and every Rust dependency's license text. Debian's packages keep
# theirs under /usr/share/doc.
COPY --from=build /src/LICENSE /src/THIRD_PARTY_NOTICES.md /src/dependency-licenses.txt \
    /usr/share/doc/crtsim-service/
USER crtsim
WORKDIR /home/crtsim
# lavapipe and FFmpeg want somewhere to put their sockets and caches.
ENV XDG_RUNTIME_DIR=/tmp
EXPOSE 8080
# Inside the container it listens on every address; publish the port only where the token
# keeps it.
ENTRYPOINT ["crtsim-service", "--bind", "0.0.0.0:8080", "--backend", "vulkan"]
