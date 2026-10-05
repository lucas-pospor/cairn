# Environment for Android builds. Source it:  . scripts/android-env.sh
# Adjust the paths if your JDK, SDK or NDK live elsewhere.
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
export NDK_HOME="${NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)}"
if [ -z "$JAVA_HOME" ]; then
  for j in /usr/lib/jvm/java-21-openjdk /usr/lib/jvm/java-17-openjdk; do
    [ -x "$j/bin/java" ] && export JAVA_HOME="$j" && break
  done
fi
# rustup's cargo (it has the Android targets) must win over a distro cargo.
export PATH="$HOME/.cargo/bin:$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"
