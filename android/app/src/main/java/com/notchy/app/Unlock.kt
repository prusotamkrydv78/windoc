package com.notchy.app

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.net.wifi.WifiManager
import android.os.Bundle
import android.os.CancellationSignal
import android.util.Base64
import android.widget.Toast
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Unlock the PC with this phone's fingerprint. The PC's "Unlock with phone" sign-in tile
 * (crates/notchy-unlock) broadcasts `NOTCHY-UNLOCK?<pair id> <nonce> <pc name>` on UDP
 * 47823; this shows a notification, checks the fingerprint, and answers with the nonce
 * sealed under the Notchy Link key. Works on the lock screen and after a cold boot, since
 * Notchy itself is not running there.
 */
object UnlockWatcher {
    const val PORT = 47823
    private const val CHANNEL = "unlock"
    private const val NOTIFICATION = 40

    @Volatile private var running = false
    private var lastNonce: String? = null
    private var lock: WifiManager.MulticastLock? = null

    fun start(context: Context) {
        if (running) return
        running = true
        val app = context.applicationContext
        // Some phones drop broadcasts unless an app holds a multicast lock.
        lock = app.getSystemService(WifiManager::class.java).createMulticastLock("notchy-unlock").apply {
            setReferenceCounted(false)
            runCatching { acquire() }
        }
        Thread({ loop(app) }, "notchy-unlock").apply { isDaemon = true; start() }
    }

    fun stop() {
        running = false
        runCatching { lock?.release() }
    }

    private fun loop(context: Context) {
        val socket = runCatching {
            DatagramSocket(null).apply {
                reuseAddress = true
                broadcast = true
                bind(InetSocketAddress(PORT))
                soTimeout = 2000
            }
        }.getOrNull() ?: return
        socket.use {
            val buffer = ByteArray(512)
            while (running) {
                val packet = DatagramPacket(buffer, buffer.size)
                try {
                    socket.receive(packet)
                } catch (_: Exception) {
                    continue
                }
                val text = String(buffer, 0, packet.length)
                if (!text.startsWith("NOTCHY-UNLOCK?")) continue
                val parts = text.removePrefix("NOTCHY-UNLOCK?").split(" ", limit = 3)
                if (parts.size < 2) continue
                val pairing = Link.pairing(context) ?: continue
                if (parts[0] != pairId(pairing.key)) continue
                val nonce = parts[1]
                if (nonce == lastNonce) continue
                lastNonce = nonce
                val pcName = parts.getOrNull(2) ?: pairing.pcName
                ask(context, packet.address.hostAddress ?: continue, packet.port, nonce, pcName)
            }
        }
    }

    fun pairId(key: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(key).take(8).joinToString("") { "%02x".format(it) }

    private fun ask(context: Context, host: String, port: Int, nonce: String, pcName: String) {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Unlock PC", NotificationManager.IMPORTANCE_HIGH),
        )
        val intent = Intent(context, UnlockActivity::class.java)
            .putExtra("host", host).putExtra("port", port)
            .putExtra("nonce", nonce).putExtra("pc", pcName)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val open = PendingIntent.getActivity(
            context, 5, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = Notification.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notchy)
            .setContentTitle("Unlock $pcName?")
            .setContentText("Tap and touch the fingerprint sensor")
            .setContentIntent(open)
            .setAutoCancel(true)
            .setCategory(Notification.CATEGORY_CALL)
            .setTimeoutAfter(120_000)
            .build()
        manager.notify(NOTIFICATION, notification)
        RobotService.instance?.say("$pcName wants in")
    }

    fun clear(context: Context) =
        context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION)

    /** Seals `ok:<nonce>` and sends it to the PC a few times (UDP may drop one). */
    fun approve(context: Context, host: String, port: Int, nonce: String): Boolean {
        val key = Link.pairing(context)?.key ?: return false
        val iv = ByteArray(12).also(SecureRandom()::nextBytes)
        val cipher = Cipher.getInstance("ChaCha20-Poly1305").apply {
            init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "ChaCha20"), IvParameterSpec(iv))
        }
        val sealed = iv + cipher.doFinal("ok:$nonce".toByteArray())
        val message = ("NOTCHY-UNLOCK!" + Base64.encodeToString(sealed, Base64.NO_WRAP)).toByteArray()
        return runCatching {
            DatagramSocket().use { socket ->
                val target = InetAddress.getByName(host)
                repeat(3) {
                    socket.send(DatagramPacket(message, message.size, target, port))
                    Thread.sleep(150)
                }
            }
            true
        }.getOrDefault(false)
    }
}

/** Asks for the fingerprint, then tells the PC's sign-in tile to unlock. */
class UnlockActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val host = intent.getStringExtra("host")
        val nonce = intent.getStringExtra("nonce")
        val port = intent.getIntExtra("port", UnlockWatcher.PORT)
        val pc = intent.getStringExtra("pc") ?: "PC"
        if (host == null || nonce == null) {
            finish()
            return
        }
        val prompt = BiometricPrompt.Builder(this)
            .setTitle("Unlock $pc")
            .setSubtitle("Notchy will sign you in on the PC")
            .setAllowedAuthenticators(
                BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL,
            )
            .build()
        prompt.authenticate(CancellationSignal(), mainExecutor, object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                UnlockWatcher.clear(this@UnlockActivity)
                Thread {
                    val sent = UnlockWatcher.approve(this@UnlockActivity, host, port, nonce)
                    runOnUiThread {
                        Toast.makeText(
                            this@UnlockActivity,
                            if (sent) "Unlocking $pc…" else "Couldn't reach $pc",
                            Toast.LENGTH_SHORT,
                        ).show()
                        RobotService.instance?.say(if (sent) "Welcome back!" else "Can't reach PC")
                        finish()
                    }
                }.start()
            }

            override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                finish()
            }
        })
    }
}
