package org.sigil.materials

import android.os.Bundle
import android.view.MotionEvent
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.locks.LockSupport

object Native {
    init { System.loadLibrary("sigil_material_android") }
    @JvmStatic external fun create(surface: Surface, width: Int, height: Int): String
    @JvmStatic external fun frame(kind: Int, die: Int, font: Int, mode: Int, border: Int, action: Int, sequence: Int, yaw: Float, pitch: Float, reduced: Int, quality: Int): String
    @JvmStatic external fun destroy()
}
data class Controls(val kind: Int = 0, val die: Int = 1, val font: Int = 0, val mode: Int = 0, val border: Int = 2, val action: Int = 0, val sequence: Int = 0, val yaw: Float = 0f, val pitch: Float = 0f, val reduced: Boolean = false, val quality: Boolean = false)

class MaterialSurface(context: android.content.Context, private val status: (String) -> Unit) : SurfaceView(context), SurfaceHolder.Callback {
    private val controls = AtomicReference(Controls())
    @Volatile private var alive = false
    private var worker: Thread? = null
    private var resumed = true
    private var bufferWidth = 0
    private var bufferHeight = 0
    private var previousX = 0f
    private var previousY = 0f
    init { holder.addCallback(this) }
    fun update(value: Controls) {
        controls.updateAndGet { value.copy(yaw = it.yaw, pitch = it.pitch) }
        LockSupport.unpark(worker)
    }
    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_MOVE) {
            val dx = (event.x - previousX) / width * 5f
            val dy = (event.y - previousY) / height * 5f
            controls.updateAndGet { it.copy(yaw = (it.yaw + dx).coerceIn(-20f,20f), pitch = (it.pitch + dy).coerceIn(-20f,20f)) }
            LockSupport.unpark(worker)
        }
        previousX = event.x; previousY = event.y
        return true
    }
    override fun surfaceCreated(holder: SurfaceHolder) { fitBuffer(width,height) }
    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) { super.onSizeChanged(w,h,oldw,oldh); fitBuffer(w,h) }
    private fun fitBuffer(w: Int, h: Int) {
        if (w <= 0 || h <= 0) return
        val ratio = h.toFloat() / w
        bufferWidth = minOf(512, (768 / ratio).toInt()).coerceAtLeast(1)
        bufferHeight = (bufferWidth * ratio).toInt().coerceIn(1,768)
        holder.setFixedSize(bufferWidth,bufferHeight)
    }
    fun pause() { resumed = false; stop() }
    fun resume() { resumed = true; if (holder.surface.isValid && worker == null) surfaceChanged(holder,0,bufferWidth,bufferHeight) }
    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        stop()
        if (!resumed || width != bufferWidth || height != bufferHeight) return
        alive = true
        worker = Thread({
            try {
                android.util.Log.i("SigilMaterials", "Viewport ${width}x${height}")
                val ready = Native.create(holder.surface, width, height)
                if (ready.startsWith("error|")) { post { status(ready.substringAfter('|')) }; return@Thread }
                var previous: Controls? = null
                var running = false
                var frames = 0
                var windowStart = System.nanoTime()
                while (alive) {
                    val now = controls.get()
                    if (previous == now && !running) { LockSupport.parkNanos(100_000_000); continue }
                    if (!running) { frames = 0; windowStart = System.nanoTime() }
                    val result = Native.frame(now.kind, now.die, now.font, now.mode, now.border, now.action, now.sequence, now.yaw, now.pitch, if (now.reduced) 1 else 0, if (now.quality) 1 else 0)
                    if (result.startsWith("error|")) { post { status(result.substringAfter('|')) }; break }
                    running = result.startsWith("1|")
                    frames++
                    val elapsed = (System.nanoTime() - windowStart) / 1e9
                    if (elapsed > 0.5 || !running) {
                        val text = result.substringAfter('|') + if (running) " · %.0f fps submitted".format(frames / elapsed) else " · Drag to inspect"
                        if (running) android.util.Log.i("SigilMaterials", text)
                        post { status(text) }
                        frames = 0; windowStart = System.nanoTime()
                    }
                    previous = now
                }
            } catch (e: Exception) {
                post { status("Renderer: ${e.message}") }
            } finally { Native.destroy() }
        }, "Sigil material renderer").also { it.start() }
    }
    private fun stop() { alive = false; LockSupport.unpark(worker); worker?.join(); worker = null }
    override fun surfaceDestroyed(holder: SurfaceHolder) { stop() }
}
class MainActivity : ComponentActivity() {
    private var materialSurface: MaterialSurface? = null
    override fun onPause() { materialSurface?.pause(); super.onPause() }
    override fun onResume() { super.onResume(); materialSurface?.resume() }
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme(colorScheme = darkColorScheme(primary = Color(0xffd9d5df), secondaryContainer = Color(0xff444448), onSecondaryContainer = Color.White, onPrimary = Color(0xff171719), surface = Color(0xff171719), background = Color(0xff101012)), shapes = Shapes(small = RoundedCornerShape(18.dp), medium = RoundedCornerShape(24.dp))) {
                var controls by remember { mutableStateOf(Controls()) }
                var status by remember { mutableStateOf("Starting Vulkan…") }
                Surface(Modifier.fillMaxSize()) {
                    Column(Modifier.fillMaxSize().systemBarsPadding().padding(horizontal = 12.dp)) {
                        Text("Sigil Materials", style = MaterialTheme.typography.headlineMedium, modifier = Modifier.padding(vertical = 8.dp))
                        Choices(listOf("Dice", "Card", "Coin"), controls.kind) { controls = controls.copy(kind = it) }
                        if (controls.kind == 0) Choices(listOf("d4", "d6", "d8", "d10", "d12", "d20"), controls.die) { controls = controls.copy(die = it) }
                        if (controls.kind == 1) Choices(listOf("Classic", "Petal", "Guilloché"), controls.border) { controls = controls.copy(border = it) }
                        Choices(listOf("Newsreader", "Google Sans Flex"), controls.font) { controls = controls.copy(font = it) }
                        Choices(listOf("Personalized", "Global", "Conversation"), controls.mode) { controls = controls.copy(mode = it) }
                        Choices(listOf("Balanced · 1× moving / 4× still", "Quality · 4×"), if (controls.quality) 1 else 0) { controls = controls.copy(quality = it == 1) }
                        AndroidView(factory = { context -> MaterialSurface(context) { status = it }.also { materialSurface = it } }, update = { it.update(controls) }, modifier = Modifier.fillMaxWidth().weight(1f), onRelease = { it.pause(); if (materialSurface === it) materialSurface = null })
                        Text(status, style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(vertical = 6.dp))
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(shape = RoundedCornerShape(18.dp), onClick = { controls = controls.copy(action = 1, sequence = controls.sequence + 1) }) { Text(if (controls.kind == 0) "Roll" else "Flip") }
                            Button(shape = RoundedCornerShape(18.dp), onClick = { controls = controls.copy(action = 2, sequence = controls.sequence + 1) }) { Text("Replay") }
                            Button(shape = RoundedCornerShape(18.dp), onClick = { controls = controls.copy(action = 3, sequence = controls.sequence + 1) }) { Text("Inspect") }
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Checkbox(checked = controls.reduced, onCheckedChange = { controls = controls.copy(reduced = it) })
                            Text("Reduced motion", modifier = Modifier.padding(top = 12.dp))
                        }
                        Text("Synthetic throws · sample collision boundaries", style = MaterialTheme.typography.labelSmall, modifier = Modifier.padding(bottom = 8.dp))
                    }
                }
            }
        }
    }
}
@Composable private fun Choices(labels: List<String>, selected: Int, change: (Int) -> Unit) {
    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        labels.forEachIndexed { index, label -> FilterChip(shape = RoundedCornerShape(14.dp), selected = index == selected, onClick = { change(index) }, label = { Text(label) }) }
    }
}
