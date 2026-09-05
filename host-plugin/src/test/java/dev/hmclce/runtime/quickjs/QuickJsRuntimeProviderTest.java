package dev.hmclce.runtime.quickjs;

import org.jackhuang.hmcl.plugin.PluginArtifactIdentity;
import org.jackhuang.hmcl.plugin.PluginDataObject;
import org.jackhuang.hmcl.plugin.PluginDataValue;
import org.jackhuang.hmcl.plugin.PluginHookEvent;
import org.jackhuang.hmcl.plugin.PluginHookPoint;
import org.jackhuang.hmcl.plugin.PluginHookResult;
import org.jackhuang.hmcl.plugin.PluginSecretAccess;
import org.jackhuang.hmcl.plugin.bridge.BridgeValue;
import org.jackhuang.hmcl.plugin.bridge.PluginCapabilityToken;
import org.jackhuang.hmcl.plugin.bridge.PluginPermissionAuthority;
import org.jackhuang.hmcl.plugin.bridge.RuntimeBridgeWireCodec;
import org.jackhuang.hmcl.plugin.runtime.PluginExecutionMode;
import org.jackhuang.hmcl.plugin.runtime.RuntimeFeature;
import org.jackhuang.hmcl.plugin.runtime.RuntimePayloadContext;
import org.jackhuang.hmcl.plugin.runtime.RuntimePayloadHandle;
import org.jackhuang.hmcl.plugin.runtime.RuntimeProvider;
import org.jackhuang.hmcl.plugin.runtime.RuntimeProviderDeclaration;
import org.jetbrains.annotations.NotNullByDefault;
import org.jetbrains.annotations.Nullable;
import org.jetbrains.annotations.Unmodifiable;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.math.BigDecimal;
import java.nio.file.Path;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertInstanceOf;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;

/// Verifies Provider ownership and delegation to one Aura process session per payload.
@NotNullByDefault
final class QuickJsRuntimeProviderTest {
    /// Test-owned package and executable root.
    @TempDir
    Path temporaryDirectory;

    /// Drives the complete isolated lifecycle and rejects a foreign handle.
    @Test
    void delegatesLifecycleAndRejectsForeignHandles() throws Exception {
        RecordingSession session = new RecordingSession();
        QuickJsRuntimeProvider provider = provider(session);

        RuntimePayloadHandle handle = provider.loadPayload(payloadContext());
        provider.enablePayload(handle);
        assertArrayEquals(new byte[]{3, 2, 1}, provider.invokePayload(handle, "echo", new byte[]{1, 2, 3}, 7));
        provider.disablePayload(handle);
        provider.unloadPayload(handle);

        assertEquals(List.of("enable", "invoke:echo:7", "disable", "shutdown"), session.events);
        RuntimePayloadHandle foreign = new RuntimePayloadHandle("payload", "foreign", handle.payloadId());
        assertThrows(IOException.class, () -> provider.enablePayload(foreign));
    }

    /// Sends one Hook through the canonical wire operation with the dispatcher's exact timeout.
    @Test
    void dispatchesHookThroughCanonicalWireWithExactTimeout() throws Exception {
        RecordingSession session = new RecordingSession();
        session.invocationResult = hookWire(mapOf(
                "contractVersion", BridgeValue.integer(1L),
                "action", BridgeValue.string("unchanged")
        ));
        QuickJsRuntimeProvider provider = provider(session);
        RuntimePayloadHandle handle = provider.loadPayload(payloadContext());
        RuntimeProvider.HookInvoker invoker = assertInstanceOf(RuntimeProvider.HookInvoker.class, provider);

        PluginHookResult result = Objects.requireNonNull(
                invoker.invokeHook(
                        handle,
                        hookToken(),
                        hookEvent(),
                        Duration.ofMillis(275L)
                ),
                "QuickJS Hook test payload returned a malformed result"
        );

        Map<String, BridgeValue> expectedData = new LinkedHashMap<>();
        expectedData.put("enabled", BridgeValue.bool(true));
        expectedData.put("attempts", BridgeValue.integer(3L));
        Map<String, BridgeValue> expectedEvent = new LinkedHashMap<>();
        expectedEvent.put("contractVersion", BridgeValue.integer(1L));
        expectedEvent.put("dispatchId", BridgeValue.string("dispatch-quickjs-42"));
        expectedEvent.put("point", BridgeValue.string("before-game-launch"));
        expectedEvent.put("occurredAt", BridgeValue.string("2026-09-05T00:00:00Z"));
        expectedEvent.put("data", BridgeValue.map(expectedData));
        assertEquals(PluginHookResult.Action.UNCHANGED, result.action());
        assertEquals("hook.before-game-launch", session.invokedOperation);
        assertEquals(0L, session.invokedCallbackId);
        assertEquals(Duration.ofMillis(275L), session.invokedTimeout);
        assertEquals(
                BridgeValue.map(expectedEvent),
                RuntimeBridgeWireCodec.decode(Objects.requireNonNull(session.invokedInput))
        );
    }

    /// Rejects invalid Hook boundaries and treats malformed child output as absent.
    @Test
    void rejectsInvalidHookBoundaryAndMalformedResult() throws Exception {
        RecordingSession session = new RecordingSession();
        session.invocationResult = new byte[]{0x01, 0x02};
        QuickJsRuntimeProvider provider = provider(session);
        RuntimePayloadHandle handle = provider.loadPayload(payloadContext());
        RuntimeProvider.HookInvoker invoker = assertInstanceOf(RuntimeProvider.HookInvoker.class, provider);

        assertNull(invoker.invokeHook(handle, hookToken(), hookEvent(), Duration.ofSeconds(1L)));
        assertThrows(NullPointerException.class,
                () -> invoker.invokeHook(handle, null, hookEvent(), Duration.ofSeconds(1L)));
        assertThrows(IllegalArgumentException.class,
                () -> invoker.invokeHook(handle, hookToken(), hookEvent(), Duration.ZERO));
        assertThrows(IllegalArgumentException.class,
                () -> invoker.invokeHook(handle, hookToken(), hookEvent(), Duration.ofMillis(-1L)));
        RuntimePayloadHandle unknown = new RuntimePayloadHandle(
                "dev.hmclce.test.javascript", provider.descriptor().providerId(), "unknown");
        assertThrows(IOException.class,
                () -> invoker.invokeHook(unknown, hookToken(), hookEvent(), Duration.ofSeconds(1L)));
    }

    /// Propagates one shared process-session transport failure without changing its identity.
    @Test
    void propagatesHookSessionFailure() throws Exception {
        RecordingSession session = new RecordingSession();
        IOException expected = new IOException("expected session failure");
        session.invocationFailure = expected;
        QuickJsRuntimeProvider provider = provider(session);
        RuntimePayloadHandle handle = provider.loadPayload(payloadContext());
        RuntimeProvider.HookInvoker invoker = assertInstanceOf(RuntimeProvider.HookInvoker.class, provider);

        IOException actual = assertThrows(IOException.class,
                () -> invoker.invokeHook(handle, hookToken(), hookEvent(), Duration.ofMillis(600L)));

        assertSame(expected, actual);
        assertEquals(Duration.ofMillis(600L), session.invokedTimeout);
    }

    /// Creates one Provider backed by the supplied recording session.
    ///
    /// @param session process-session test boundary
    /// @return isolated QuickJS Provider
    private QuickJsRuntimeProvider provider(RecordingSession session) {
        return new QuickJsRuntimeProvider(
                "dev.hmclce.runtime.quickjs-host", "0.1.0-beta.1", List.of(new RuntimeProviderDeclaration(
                        "javascript", Set.of(1), 1, Set.of(PluginExecutionMode.ISOLATED),
                        Set.of(RuntimeFeature.BRIDGE, RuntimeFeature.HOOKS, RuntimeFeature.PATCHES,
                                RuntimeFeature.NATIVE))),
                temporaryDirectory.resolve("aura-quickjs-host.exe"), (path, context) -> session);
    }

    /// Creates one payload context whose Java capability token supplier must remain unused.
    ///
    /// @return isolated payload context
    private RuntimePayloadContext payloadContext() {
        return new RuntimePayloadContext(
                new PluginArtifactIdentity("dev.hmclce.test.javascript", "1.0.0", "a".repeat(64)),
                temporaryDirectory, "aura-javascript.json", PluginExecutionMode.ISOLATED,
                temporaryDirectory.resolve("data"), () -> {
                    throw new AssertionError("token resolved");
                });
    }

    /// Creates one live opaque Java capability token.
    ///
    /// @return live token whose bytes must not cross the Provider boundary
    private static PluginCapabilityToken hookToken() {
        return new PluginPermissionAuthority().issue(
                new PluginArtifactIdentity("dev.hmclce.test.javascript", "1.0.0", "e".repeat(64)),
                PluginExecutionMode.ISOLATED,
                Set.of(),
                "runtime.payload",
                Duration.ofMinutes(1L)
        );
    }

    /// Creates one deterministic Hook event containing ordinary data and denied secret access.
    ///
    /// @return immutable Hook event
    private static PluginHookEvent hookEvent() {
        Map<String, PluginDataValue> data = new LinkedHashMap<>();
        data.put("enabled", PluginDataValue.bool(true));
        data.put("attempts", PluginDataValue.number(new BigDecimal("3")));
        return new PluginHookEvent(
                1,
                "dispatch-quickjs-42",
                PluginHookPoint.BEFORE_GAME_LAUNCH,
                Instant.parse("2026-09-05T00:00:00Z"),
                PluginDataObject.of(data),
                PluginSecretAccess.denied("dev.hmclce.test.javascript")
        );
    }

    /// Creates one insertion-ordered Bridge map from alternating keys and values.
    ///
    /// @param entries alternating string keys and Bridge values
    /// @return insertion-ordered Bridge map
    private static Map<String, BridgeValue> mapOf(Object... entries) {
        Map<String, BridgeValue> values = new LinkedHashMap<>();
        for (int index = 0; index < entries.length; index += 2) {
            values.put((String) entries[index], (BridgeValue) entries[index + 1]);
        }
        return values;
    }

    /// Encodes one hand-authored external Hook result.
    ///
    /// @param values exact result fields
    /// @return canonical Bridge Value v1 bytes
    /// @throws IOException if the fixture cannot be encoded
    private static byte[] hookWire(Map<String, BridgeValue> values) throws IOException {
        return RuntimeBridgeWireCodec.encode(BridgeValue.map(values));
    }

    /// Records the exact Provider-to-session operation order.
    @NotNullByDefault
    private static final class RecordingSession implements QuickJsRuntimeProvider.Session {
        /// Ordered delegated calls.
        private final List<String> events = new ArrayList<>();

        /// Last invoked operation, or `null` before invocation.
        private @Nullable String invokedOperation;

        /// Last invoked input bytes, or `null` before invocation.
        private byte @Nullable [] invokedInput;

        /// Last callback identifier.
        private long invokedCallbackId = Long.MIN_VALUE;

        /// Last invocation deadline, or `null` before invocation.
        private @Nullable Duration invokedTimeout;

        /// Optional scripted invocation result.
        private byte @Nullable [] invocationResult;

        /// Optional scripted transport failure.
        private @Nullable IOException invocationFailure;

        /// Records enable.
        @Override public void enable() { events.add("enable"); }

        /// Records invocation and returns either scripted output or reversed test bytes.
        @Override public byte @Unmodifiable [] invoke(
                String operation,
                byte @Unmodifiable [] input,
                long callbackId,
                Duration timeout
        ) throws IOException {
            events.add("invoke:" + operation + ":" + callbackId);
            invokedOperation = operation;
            invokedInput = input.clone();
            invokedCallbackId = callbackId;
            invokedTimeout = timeout;
            if (invocationFailure != null) {
                throw invocationFailure;
            }
            if (invocationResult != null) {
                return invocationResult.clone();
            }
            return new byte[]{input[2], input[1], input[0]};
        }

        /// Records disable.
        @Override public void disable() { events.add("disable"); }

        /// Records graceful shutdown.
        @Override public void shutdown() { events.add("shutdown"); }

        /// Records fallback close only when used.
        @Override public void close() { events.add("close"); }
    }
}
