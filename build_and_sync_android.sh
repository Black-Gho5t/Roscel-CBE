#!/usr/bin/env bash
set -e

echo "🦀 Compilando Roscel CBE para Android (x86_64 y aarch64)..."

export ANDROID_HOME="/home/yader/Android/Sdk"
export ANDROID_NDK_HOME="/home/yader/Android/Sdk/ndk/28.2.13676358"

# 1. Compilar para emuladores x86_64
cargo build --target x86_64-linux-android --release

# 2. Compilar para dispositivos reales ARM64
cargo build --target aarch64-linux-android --release

# 3. Sincronizar binarios con el proyecto destino (por defecto roscel_sandbox)
TARGET_PROJECT="${1:-/home/yader/roscel_sandbox}"
TARGET_JNI="$TARGET_PROJECT/android/app/src/main/jniLibs"

mkdir -p "$TARGET_JNI/x86_64"
mkdir -p "$TARGET_JNI/arm64-v8a"

cp target/x86_64-linux-android/release/libroscel_cbe.so "$TARGET_JNI/x86_64/libroscel_cbe.so"
cp target/aarch64-linux-android/release/libroscel_cbe.so "$TARGET_JNI/arm64-v8a/libroscel_cbe.so"

echo "✅ Binarios sincronizados exitosamente en $TARGET_JNI"
ls -lh "$TARGET_JNI"/*/*.so
