FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive \
    ANDROID_HOME=/opt/android-sdk \
    NDK_VERSION=30.0.14904198 \
    BUILD_TOOLS_VERSION=36.1.0 \
    PLATFORM=android-34 \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo

ENV ANDROID_NDK_HOME=${ANDROID_HOME}/ndk/${NDK_VERSION} \
    PATH=/usr/local/cargo/bin:$PATH

RUN apt-get update && apt-get install -y --no-install-recommends \
        openjdk-17-jdk-headless \
        wget unzip curl ca-certificates git zip python3 \
    && rm -rf /var/lib/apt/lists/*

# Android SDK command-line tools
RUN mkdir -p "$ANDROID_HOME/cmdline-tools" \
    && wget -q https://dl.google.com/android/repository/commandlinetools-linux-11076708_latest.zip \
         -O /tmp/cmdtools.zip \
    && unzip -q /tmp/cmdtools.zip -d "$ANDROID_HOME/cmdline-tools" \
    && mv "$ANDROID_HOME/cmdline-tools/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest" \
    && rm /tmp/cmdtools.zip \
    && yes | "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" --licenses >/dev/null 2>&1 || true \
    && "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" \
        "ndk;${NDK_VERSION}" \
        "build-tools;${BUILD_TOOLS_VERSION}" \
        "platforms;${PLATFORM}"

# Rust stable + aarch64-linux-android target
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain stable \
    && rustup target add aarch64-linux-android

# cargo-ndk (compiled once into the image)
RUN cargo install cargo-ndk --version 4.1.2 --locked \
    && rm -rf "$CARGO_HOME/registry" "$CARGO_HOME/git"

# sccache — pre-built musl binary, no Rust compile needed
RUN curl -sL "https://github.com/mozilla/sccache/releases/download/v0.8.1/sccache-v0.8.1-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz -C /tmp \
    && mv /tmp/sccache-v0.8.1-x86_64-unknown-linux-musl/sccache /usr/local/bin/sccache \
    && rm -rf /tmp/sccache-v0.8.1-x86_64-unknown-linux-musl \
    && chmod +x /usr/local/bin/sccache
