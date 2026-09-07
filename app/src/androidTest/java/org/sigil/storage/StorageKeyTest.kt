package org.sigil.storage

import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.security.KeyStore

class StorageKeyTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val providers = mutableListOf<StorageKeyProvider>()
    private fun provider(name: String) = StorageKeyProvider(context, "key-test-$name").also { providers.add(it) }

    @After fun cleanup() {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        providers.forEach { store.deleteEntry(it.alias); it.directory.deleteRecursively() }
    }

    @Test fun hardwareWrappedKeyReopensRustStoreAndClearsBorrowedBytes() {
        val provider = provider("reopen")
        var borrowed: ByteArray? = null
        provider.withKey { directory, key ->
            borrowed = key
            assertTrue(NativeStorage.checkStore(directory.path, key))
        }
        assertTrue(borrowed!!.all { it == 0.toByte() })
        StorageKeyProvider(context, "key-test-reopen").withKey { directory, key ->
            assertFalse(NativeStorage.checkStore(directory.path, ByteArray(32)))
            assertTrue(NativeStorage.checkStore(directory.path, key))
        }
        assertEquals(context.noBackupFilesDir.canonicalFile, provider.directory.parentFile!!.canonicalFile)
    }

    @Test fun alteredEnvelopeFailsWithoutReplacingKeyOrDatabase() {
        val provider = provider("tamper")
        provider.withKey { directory, key -> assertTrue(NativeStorage.checkStore(directory.path, key)) }
        val file = File(provider.directory, "storage.key")
        val bytes = file.readBytes().also { it[30] = (it[30].toInt() xor 1).toByte() }
        file.writeBytes(bytes)
        assertThrows(Exception::class.java) { provider.withKey { _, _ -> fail("Tampered key accepted") } }
        assertArrayEquals(bytes, file.readBytes())
        assertTrue(File(provider.directory, "client.db").exists())
    }

    @Test fun missingWrappingKeyOrEnvelopeNeverResetsExistingDatabase() {
        for ((name, removeAlias) in listOf("missing-alias" to true, "missing-file" to false)) {
            val provider = provider(name)
            provider.withKey { directory, key -> assertTrue(NativeStorage.checkStore(directory.path, key)) }
            if (removeAlias) KeyStore.getInstance("AndroidKeyStore").apply { load(null); deleteEntry(provider.alias) }
            else assertTrue(File(provider.directory, "storage.key").delete())
            assertThrows(Exception::class.java) { provider.withKey { _, _ -> fail("Missing key accepted") } }
            assertTrue(File(provider.directory, "client.db").exists())
        }
    }

    @Test fun concurrentAccessCannotReplaceAnEstablishedKey() {
        val provider = provider("concurrent")
        provider.withKey { directory, key -> assertTrue(NativeStorage.checkStore(directory.path, key)) }
        val failures = java.util.concurrent.atomic.AtomicInteger()
        provider.withKey { _, _ ->
            val worker = Thread {
                try { provider.withKey { _, _ -> failures.incrementAndGet() } }
                catch (_: java.nio.channels.OverlappingFileLockException) { /* Safe retry after lock owner exits. */ }
                catch (_: Throwable) { failures.incrementAndGet() }
            }
            worker.start(); worker.join(3000)
            assertFalse(worker.isAlive)
        }
        assertEquals(0, failures.get())
        provider.withKey { directory, key -> assertTrue(NativeStorage.checkStore(directory.path, key)) }
    }

    @Test fun callbackFailureStillClearsKeyBytes() {
        val provider = provider("callback")
        var borrowed: ByteArray? = null
        assertThrows(IllegalStateException::class.java) {
            provider.withKey { _, key -> borrowed = key; error("Synthetic callback failure") }
        }
        assertTrue(borrowed!!.all { it == 0.toByte() })
        provider.withKey { directory, key -> assertTrue(NativeStorage.checkStore(directory.path, key)) }
    }
}
