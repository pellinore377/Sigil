package org.sigil.storage

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Process
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.system.Os
import android.system.OsConstants
import android.util.AtomicFile
import java.io.File
import java.io.RandomAccessFile
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec

/** Hardware wrapping only; Rust owns database and messaging behavior. */
class StorageKeyProvider(context: Context, name: String = "native") {
    private val app = context.applicationContext
    internal val directory: File
    internal val alias: String
    private val aad: ByteArray
    private val magic = byteArrayOf(83, 71, 65, 75, 0, 1, 0, 0)

    init {
        require(name.matches(Regex("[a-z0-9_-]{1,32}")))
        check(!app.isDeviceProtectedStorage)
        directory = File(app.noBackupFilesDir, name)
        if (!directory.exists()) {
            try { Os.mkdir(directory.path, 448) } catch (error: android.system.ErrnoException) {
                if (error.errno != OsConstants.EEXIST) throw error
            }
        }
        val stat = Os.lstat(directory.path)
        check(OsConstants.S_ISDIR(stat.st_mode) && stat.st_mode and 63 == 0 && stat.st_uid == Process.myUid())
        alias = "${app.packageName}/storage/$name/v0"
        aad = "Sigil/Android/storage-key/v0\u0000${app.packageName}\u0000$name".toByteArray(Charsets.UTF_8)
    }

    /** The callback must not retain key bytes; the supplied array is cleared. */
    fun <T> withKey(block: (File, ByteArray) -> T): T {
        val lockFile = File(directory, "key.lock")
        RandomAccessFile(lockFile, "rw").use { lock ->
            Os.chmod(lockFile.path, 384)
            lock.channel.lock().use {
                val atomic = AtomicFile(File(directory, "storage.key"))
                val exists = atomic.baseFile.exists() || File(directory, "storage.key.bak").exists()
                val keyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
                val wrapping = if (keyStore.containsAlias(alias)) {
                    keyStore.getKey(alias, null) as? SecretKey ?: error("Storage wrapping key unavailable")
                } else {
                    check(!exists && !File(directory, "client.db").exists()) { "Storage wrapping key missing" }
                    generate()
                }
                requireHardware(wrapping)
                val key = if (exists) unwrap(wrapping, read(atomic)) else {
                    check(!File(directory, "client.db").exists()) { "Wrapped storage key missing" }
                    create(wrapping, atomic)
                }
                try { check(key.size == 32); return block(directory, key) } finally { key.fill(0) }
            }
        }
    }

    private fun generate(): SecretKey {
        fun generate(strongBox: Boolean): SecretKey {
            val spec = KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true).setUserAuthenticationRequired(false)
            if (Build.VERSION.SDK_INT >= 28) spec.setIsStrongBoxBacked(strongBox)
            return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").apply { init(spec.build()) }.generateKey()
        }
        if (Build.VERSION.SDK_INT >= 28 && app.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)) {
            try { return generate(true) } catch (_: StrongBoxUnavailableException) { /* TEE fallback remains hardware-only. */ }
        }
        return generate(false)
    }

    @Suppress("DEPRECATION")
    private fun requireHardware(key: SecretKey) {
        val info = SecretKeyFactory.getInstance(key.algorithm, "AndroidKeyStore").getKeySpec(key, KeyInfo::class.java) as KeyInfo
        val hardware = if (Build.VERSION.SDK_INT >= 31) {
            info.securityLevel == KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT || info.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX
        } else info.isInsideSecureHardware
        check(hardware && info.keySize == 256 && !info.isUserAuthenticationRequired) { "Hardware storage protection unavailable" }
        check(info.blockModes.contentEquals(arrayOf(KeyProperties.BLOCK_MODE_GCM)))
    }

    private fun read(file: AtomicFile): ByteArray = file.openRead().use { stream ->
        val bytes = ByteArray(69)
        var size = 0
        while (size < bytes.size) {
            val read = stream.read(bytes, size, bytes.size - size)
            if (read == -1) break
            check(read > 0)
            size += read
        }
        check(size == 68 && bytes.copyOfRange(0, 8).contentEquals(magic)) { "Invalid wrapped storage key" }
        bytes.copyOf(size)
    }

    private fun unwrap(key: SecretKey, bytes: ByteArray): ByteArray = Cipher.getInstance("AES/GCM/NoPadding").run {
        init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, bytes.copyOfRange(8, 20)))
        updateAAD(aad)
        doFinal(bytes, 20, 48)
    }

    private fun create(wrapping: SecretKey, file: AtomicFile): ByteArray {
        val key = ByteArray(32).also { SecureRandom().nextBytes(it) }
        try {
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, wrapping)
            cipher.updateAAD(aad)
            val sealed = cipher.doFinal(key)
            check(cipher.iv.size == 12 && sealed.size == 48)
            val record = magic + cipher.iv + sealed
            val output = file.startWrite()
            try {
                Os.fchmod(output.fd, 384)
                output.write(record)
                output.fd.sync()
                file.finishWrite(output)
            } catch (error: Throwable) { file.failWrite(output); throw error }
            val descriptor = Os.open(directory.path, OsConstants.O_RDONLY, 0)
            try { check(OsConstants.S_ISDIR(Os.fstat(descriptor).st_mode)); Os.fsync(descriptor) } finally { Os.close(descriptor) }
            check(MessageDigest.isEqual(read(file), record)) { "Storage key commit failed" }
            return key
        } catch (error: Throwable) { key.fill(0); throw error }
    }
}
