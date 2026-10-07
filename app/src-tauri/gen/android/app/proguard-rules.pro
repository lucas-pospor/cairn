# Add project specific ProGuard rules here.
# You can control the set of applied configuration files using the
# proguardFiles setting in build.gradle.
#
# For more details, see
#   http://developer.android.com/guide/developing/tools/proguard.html

# If your project uses WebView with JS, uncomment the following
# and specify the fully qualified class name to the JavaScript interface
# class:
#-keepclassmembers class fqcn.of.javascript.interface.for.webview {
#   public *;
#}

# Uncomment this to preserve the line number information for
# debugging stack traces.
#-keepattributes SourceFile,LineNumberTable

# If you keep the line number information, uncomment this to
# hide the original source file name.
#-renamesourcefileattribute SourceFile
# Cairn: the Storage Access Framework plugin is loaded by name from Rust
# (register_android_plugin) and its commands are found by reflection.
-keep class app.cairn.notes.SafPlugin { *; }
-keep class app.cairn.notes.PathArgs { *; }
-keep class app.cairn.notes.WriteArgs { *; }
-keep class app.cairn.notes.MoveArgs { *; }
# The local network permission plugin, likewise (sync, android.rs allow_server).
-keep class app.cairn.notes.LocalNetworkPlugin { *; }
-keep class app.cairn.notes.LocalNetworkArgs { *; }
# The page calls CairnAndroid.leave() (MainActivity.BackBridge) by name.
-keepclassmembers class app.cairn.notes.MainActivity$BackBridge {
    @android.webkit.JavascriptInterface <methods>;
}
