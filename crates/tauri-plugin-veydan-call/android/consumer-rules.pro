# The system starts these classes by name, from the manifest.
-keep class net.veydan.call.CallService { *; }
-keep class net.veydan.call.CallActionReceiver { *; }
-keep class net.veydan.call.IncomingCallActivity { *; }
# The engine of calls reaches the Java side of libwebrtc over JNI, by name
# (Engine.kt; crates/messenger/rtc/src/android.rs), and the camera's frames
# and the engine's init are native methods of these two. The JNI glue of
# libwebrtc (jni_zero, in the same jar) is reached by name as well, at the
# engine's init, and nothing in Java refers to it: without the rule R8
# drops it and the init aborts the process (`java_class == null`). The jar
# lacks the generated `JniZeroJni` that `JniZero.setJniClassLoader` names;
# nothing calls that method, so the missing class is not an error.
-keep class livekit.org.webrtc.** { *; }
-keep class livekit.org.jni_zero.** { *; }
-dontwarn livekit.org.jni_zero.JniZeroJni
-keep class net.veydan.call.Engine { *; }
-keep class net.veydan.call.Camera { *; }
