package dev.hmclce.runtime.quickjs;

import org.jackhuang.hmcl.plugin.PluginHookEvent;
import org.jackhuang.hmcl.plugin.PluginHookResult;
import org.jackhuang.hmcl.plugin.bridge.PluginCapabilityToken;
import org.jackhuang.hmcl.plugin.runtime.RuntimePayloadContext;
import org.jackhuang.hmcl.plugin.runtime.RuntimePayloadHandle;
import org.jackhuang.hmcl.plugin.runtime.RuntimeProvider;
import org.jackhuang.hmcl.plugin.runtime.RuntimeProviderDeclaration;
import org.jackhuang.hmcl.plugin.runtime.RuntimeProviderDescriptor;
import org.jackhuang.hmcl.plugin.runtime.RuntimeHookWireCodec;
import org.jackhuang.hmcl.plugin.runtime.process.RuntimeProcessSession;
import org.jetbrains.annotations.NotNullByDefault;
import org.jetbrains.annotations.Nullable;
import org.jetbrains.annotations.Unmodifiable;

import java.io.IOException;
import java.nio.file.Path;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.UUID;

/// Provides isolated QuickJS payload execution through Aura's shared process supervisor.
@NotNullByDefault
public final class QuickJsRuntimeProvider implements RuntimeProvider, RuntimeProvider.HookInvoker {
    /// Default deadline for generic payload invocation.
    private static final Duration INVOCATION_TIMEOUT = Duration.ofSeconds(10L);

    /// Immutable runtime declaration published to Aura.
    private final RuntimeProviderDescriptor descriptor;

    /// Exact platform process Host executable.
    private final Path executable;

    /// Starts one process session per payload.
    private final SessionFactory sessionFactory;

    /// Active sessions indexed by Provider-owned opaque IDs.
    private final Map<String, Session> sessions = new LinkedHashMap<>();

    /// Creates one isolated QuickJS Provider.
    ///
    /// @param providerId exact Host plugin ID
    /// @param version exact Host plugin version
    /// @param declarations manifest runtime declarations
    /// @param executable platform process Host executable
    /// @param sessionFactory process session boundary
    QuickJsRuntimeProvider(
            String providerId,
            String version,
            @Unmodifiable List<RuntimeProviderDeclaration> declarations,
            Path executable,
            SessionFactory sessionFactory
    ) {
        descriptor = new RuntimeProviderDescriptor(providerId, version, declarations, true, true, 100, false);
        this.executable = executable;
        this.sessionFactory = sessionFactory;
    }

    /// Returns the manifest-matched Provider descriptor.
    ///
    /// @return immutable Provider descriptor
    @Override
    public RuntimeProviderDescriptor descriptor() {
        return descriptor;
    }

    /// Reports that the Provider can start isolated processes.
    ///
    /// @return always true after Host package validation
    @Override
    public boolean healthCheck() {
        return true;
    }

    /// Starts and loads one isolated payload process.
    ///
    /// @param context exact launcher payload context retained by Aura supervision
    /// @return Provider-owned opaque payload handle
    /// @throws IOException if process startup or protocol negotiation fails
    @Override
    public synchronized RuntimePayloadHandle loadPayload(RuntimePayloadContext context) throws IOException {
        Session session = sessionFactory.start(executable, context);
        String payloadId = UUID.randomUUID().toString();
        sessions.put(payloadId, session);
        return new RuntimePayloadHandle(
                context.artifactIdentity().getPluginId(), descriptor.providerId(), payloadId);
    }

    /// Enables one loaded payload.
    ///
    /// @param handle Provider-owned payload handle
    /// @throws IOException if ownership or child lifecycle validation fails
    @Override
    public synchronized void enablePayload(RuntimePayloadHandle handle) throws IOException {
        requireSession(handle).enable();
    }

    /// Disables one enabled payload.
    ///
    /// @param handle Provider-owned payload handle
    /// @throws IOException if ownership or child lifecycle validation fails
    @Override
    public synchronized void disablePayload(RuntimePayloadHandle handle) throws IOException {
        requireSession(handle).disable();
    }

    /// Invokes one operation through the shared process protocol.
    ///
    /// @param handle Provider-owned payload handle
    /// @param operation canonical payload operation
    /// @param input canonical Bridge Value v1 bytes
    /// @param callbackId nonnegative payload callback identifier
    /// @return canonical Bridge Value v1 result bytes
    /// @throws IOException if ownership, transport, deadline, or payload execution fails
    @Override
    public synchronized byte @Unmodifiable [] invokePayload(
            RuntimePayloadHandle handle,
            String operation,
            byte @Unmodifiable [] input,
            long callbackId
    ) throws IOException {
        return requireSession(handle).invoke(operation, input, callbackId, INVOCATION_TIMEOUT);
    }

    /// Invokes one Hook through Aura's canonical language-neutral wire contract.
    ///
    /// The Java capability token is checked for presence but never serialized. The shared process session receives
    /// the dispatcher's exact positive deadline and owns enforcement of that deadline.
    ///
    /// @param handle Provider-owned payload handle
    /// @param token opaque short-lived launcher capability token
    /// @param event immutable Hook event
    /// @param timeout positive dispatcher callback deadline
    /// @return decoded Hook result, or `null` for malformed process output
    /// @throws IOException if ownership, event encoding, transport, or payload execution fails
    @Override
    public synchronized @Nullable PluginHookResult invokeHook(
            RuntimePayloadHandle handle,
            PluginCapabilityToken token,
            PluginHookEvent event,
            Duration timeout
    ) throws IOException {
        Objects.requireNonNull(token, "token");
        Duration deadline = Objects.requireNonNull(timeout, "timeout");
        if (deadline.isZero() || deadline.isNegative()) {
            throw new IllegalArgumentException("QuickJS Runtime Hook timeout must be positive");
        }
        return RuntimeHookWireCodec.decodeResult(requireSession(handle).invoke(
                RuntimeHookWireCodec.operation(event.point()),
                RuntimeHookWireCodec.encodeEvent(event),
                0L,
                deadline
        ));
    }

    /// Gracefully shuts down and removes one payload session.
    ///
    /// @param handle Provider-owned payload handle
    /// @throws IOException if ownership or shutdown fails
    @Override
    public synchronized void unloadPayload(RuntimePayloadHandle handle) throws IOException {
        Session session = requireSession(handle);
        sessions.remove(handle.payloadId());
        try {
            session.shutdown();
        } catch (IOException exception) {
            session.close();
            throw exception;
        }
    }

    /// Closes all remaining payload processes and clears Provider state.
    @Override
    public synchronized void close() {
        for (Session session : sessions.values()) {
            session.close();
        }
        sessions.clear();
    }

    /// Resolves a payload only when the handle belongs to this exact Provider.
    ///
    /// @param handle candidate payload handle
    /// @return active session
    /// @throws IOException if ownership or payload identity is invalid
    private Session requireSession(RuntimePayloadHandle handle) throws IOException {
        if (!descriptor.providerId().equals(handle.providerId())) {
            throw new IOException("QuickJS Runtime Host received a foreign payload handle: " + handle.providerId());
        }
        @Nullable Session session = sessions.get(handle.payloadId());
        if (session == null) {
            throw new IOException("QuickJS Runtime Host received an unknown payload handle");
        }
        return session;
    }

    /// Opens one process session through an injectable test boundary.
    @FunctionalInterface
    @NotNullByDefault
    interface SessionFactory {
        /// Starts and loads one session.
        ///
        /// @param executable exact process Host executable
        /// @param context exact payload context
        /// @return loaded session
        /// @throws IOException if startup fails
        Session start(Path executable, RuntimePayloadContext context) throws IOException;
    }

    /// Minimal shared process-session lifecycle used by the Provider.
    @NotNullByDefault
    interface Session extends AutoCloseable {
        /// Enables the loaded payload.
        void enable() throws IOException;

        /// Invokes one enabled payload operation.
        ///
        /// @param operation canonical operation
        /// @param input canonical Bridge Value bytes
        /// @param callbackId payload callback identifier
        /// @param timeout positive operation deadline
        /// @return canonical Bridge Value bytes
        byte @Unmodifiable [] invoke(
                String operation,
                byte @Unmodifiable [] input,
                long callbackId,
                Duration timeout
        ) throws IOException;

        /// Disables the enabled payload.
        void disable() throws IOException;

        /// Gracefully shuts down the payload.
        void shutdown() throws IOException;

        /// Terminates the process without throwing.
        @Override
        void close();
    }

    /// Adapts Aura's final shared process supervisor to the testable session boundary.
    @NotNullByDefault
    static final class AuraSession implements Session {
        /// Shared Aura process session.
        private final RuntimeProcessSession delegate;

        /// Creates one adapter.
        ///
        /// @param delegate shared Aura process session
        private AuraSession(RuntimeProcessSession delegate) {
            this.delegate = delegate;
        }

        /// Starts one exact executable and payload context.
        ///
        /// @param executable process Host executable
        /// @param context payload context
        /// @return loaded adapter
        /// @throws IOException if startup fails
        static Session start(Path executable, RuntimePayloadContext context) throws IOException {
            return new AuraSession(RuntimeProcessSession.start(executable, context));
        }

        /// Delegates enable.
        @Override public void enable() throws IOException { delegate.enable(); }

        /// Delegates invocation.
        @Override public byte @Unmodifiable [] invoke(
                String operation, byte @Unmodifiable [] input, long callbackId, Duration timeout) throws IOException {
            return delegate.invoke(operation, input, callbackId, timeout);
        }

        /// Delegates disable.
        @Override public void disable() throws IOException { delegate.disable(); }

        /// Delegates graceful shutdown.
        @Override public void shutdown() throws IOException { delegate.shutdown(); }

        /// Delegates idempotent termination.
        @Override public void close() { delegate.close(); }
    }
}
