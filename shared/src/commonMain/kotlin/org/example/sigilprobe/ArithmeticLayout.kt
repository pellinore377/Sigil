package org.sigil

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.AlignmentLine
import androidx.compose.ui.layout.FirstBaseline
import androidx.compose.ui.layout.LastBaseline
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.Placeable
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import org.jetbrains.compose.resources.Font
import sigil.shared.generated.resources.*
import androidx.compose.ui.unit.Constraints
import kotlin.math.max
import kotlin.math.roundToInt

// Arithmetic as text/src/numeric.rs reads it; presentation only, the wire keeps the source.
internal sealed interface Arith {
    data class Num(val digits: String) : Arith
    data class Sci(val mantissa: String, val exponent: String) : Arith
    data class Sign(val minus: Boolean, val value: Arith) : Arith
    data class Group(val value: Arith) : Arith
    data class Op(val op: Char, val left: Arith, val right: Arith) : Arith
}

// Same grammar and precedence as the Rust calculator: unary sits under ^, ^ is right-associative.
internal fun parseArith(source: String): Arith? {
    var at = 0
    var steps = 0
    fun space() { while (at < source.length && source[at].isWhitespace()) at++ }
    fun digits() { while (at < source.length && source[at].isDigit()) at++ }
    fun number(): Arith? {
        val start = at
        while (at < source.length && (source[at].isDigit() || source[at] == '.')) at++
        val mantissa = source.substring(start, at)
        if (source.getOrNull(at) == 'e' || source.getOrNull(at) == 'E') {
            val mark = ++at
            if (source.getOrNull(at) == '+' || source.getOrNull(at) == '-') at++
            digits()
            val exponent = source.substring(mark, at)
            source.substring(start, at).toDoubleOrNull() ?: return null
            return Arith.Sci(mantissa, exponent.removePrefix("+"))
        }
        mantissa.toDoubleOrNull() ?: return null
        return Arith.Num(mantissa)
    }
    fun expr(min: Int, depth: Int): Arith? {
        if (depth > 32 || steps++ >= 256) return null
        space()
        val first = source.getOrNull(at) ?: return null
        var left: Arith = when (first) {
            '+', '-' -> { at++; Arith.Sign(first == '-', expr(5, depth + 1) ?: return null) }
            '(' -> {
                at++
                val inner = expr(0, depth + 1) ?: return null
                space()
                if (source.getOrNull(at) != ')') return null
                at++
                Arith.Group(inner)
            }
            else -> number() ?: return null
        }
        while (true) {
            space()
            val op = source.getOrNull(at) ?: break
            val (l, r) = when (op) { '+', '-' -> 1 to 2; '*', '/', '%' -> 3 to 4; '^' -> 6 to 5; else -> break }
            if (l < min) break
            at++
            left = Arith.Op(op, left, expr(r, depth + 1) ?: return null)
        }
        return left
    }
    val tree = expr(0, 0)
    space()
    return tree?.takeIf { at == source.length }
}

private fun operatorGlyph(op: Char) = when (op) { '+' -> "+"; '-' -> "−"; '*' -> "×"; '/' -> "÷"; else -> "mod" }
private fun operatorWord(op: Char) = when (op) { '+' -> "plus"; '-' -> "minus"; '*' -> "times"; '/' -> "divided by"; else -> "modulo" }
private fun signedDigits(raw: String) = if (raw.startsWith("-")) "−" + raw.drop(1) else raw

// Words a screen reader can say unambiguously: brackets drawn are named, a compound power is closed with "end power".
internal fun arithSpoken(node: Arith, bracket: Boolean = false): String = when (node) {
    is Arith.Num -> readableNumber(node.digits)
    is Arith.Sci -> "${readableNumber(node.mantissa)} times 10 to the power of ${spokenFigure(signedDigits(node.exponent))}"
        .let { if (bracket) "open paren $it close paren" else it }
    is Arith.Sign -> (if (node.minus) "minus " else "plus ") + arithSpoken(node.value)
    is Arith.Group -> "open paren ${arithSpoken(node.value)} close paren"
    is Arith.Op -> if (node.op == '^') {
        val power = node.right
        val simple = power is Arith.Num || (power is Arith.Sign && power.value is Arith.Num)
        arithSpoken(node.left, bracket = true) + when {
            power is Arith.Num && power.digits == "2" -> " squared"
            power is Arith.Num && power.digits == "3" -> " cubed"
            simple -> " to the power of " + arithSpoken(power)
            else -> " to the power of " + arithSpoken(power) + ", end power"
        }
    } else "${arithSpoken(node.left)} ${operatorWord(node.op)} ${arithSpoken(node.right, bracket = node.op == '/' || node.op == '%')}"
}

internal fun spokenFigure(figure: String) = if (figure.startsWith("−")) "minus " + figure.drop(1) else figure

// A run of glyphs, or a base with its raised exponent; gap is operator spacing, breakable marks a legal line break.
internal sealed interface ArithToken {
    val gap: Boolean
    val breakable: Boolean
    data class Word(val text: String, override val gap: Boolean = false, override val breakable: Boolean = false, val operator: Boolean = false) : ArithToken
    data class Raise(val base: List<ArithToken>, val exponent: List<ArithToken>, override val gap: Boolean = false, override val breakable: Boolean = false) : ArithToken
}

// Numbers longer than this may break after a grouping comma.
private const val LongNumber = 12

// Flattens the tree; a line may break before a binary operator or inside a long number, never inside an exponent.
internal fun arithTokens(root: Arith, wrap: Boolean = true): List<ArithToken> {
    val out = mutableListOf<ArithToken>()
    var gap = false
    fun push(token: ArithToken) {
        val spaced = when (token) { is ArithToken.Word -> token.copy(gap = token.gap || gap); is ArithToken.Raise -> token.copy(gap = gap) }
        gap = false
        val last = out.lastOrNull()
        if (last is ArithToken.Word && spaced is ArithToken.Word && !last.operator && !spaced.operator && !spaced.gap && !spaced.breakable) out[out.lastIndex] = last.copy(text = last.text + spaced.text)
        else out += spaced
    }
    fun emit(node: Arith, bracket: Boolean = false) {
        when (node) {
            is Arith.Num -> {
                val shown = readableNumber(node.digits)
                if (!wrap || shown.length <= LongNumber) push(ArithToken.Word(shown))
                else Regex("""[^,]*,?""").findAll(shown).map { it.value }.filter { it.isNotEmpty() }
                    .forEachIndexed { n, chunk -> push(ArithToken.Word(chunk, breakable = n > 0)) }
            }
            is Arith.Sci -> {
                if (bracket) push(ArithToken.Word("("))
                push(ArithToken.Word(readableNumber(node.mantissa)))
                push(ArithToken.Word("×", gap = true, operator = true)); gap = true
                push(ArithToken.Raise(listOf(ArithToken.Word("10")), listOf(ArithToken.Word(signedDigits(node.exponent)))))
                if (bracket) push(ArithToken.Word(")"))
            }
            is Arith.Sign -> { push(ArithToken.Word(if (node.minus) "−" else "+", operator = true)); emit(node.value) }
            is Arith.Group -> { push(ArithToken.Word("(")); emit(node.value); push(ArithToken.Word(")")) }
            is Arith.Op -> if (node.op == '^') {
                // Only the base's last glyph run carries the power, so a powered group still wraps.
                emit(node.left, bracket = true)
                val last = out.removeAt(out.lastIndex)
                out += ArithToken.Raise(listOf(if (last is ArithToken.Word) last.copy(gap = false, breakable = false) else last), arithTokens(node.right, false), gap = last.gap, breakable = last.breakable)
            } else {
                emit(node.left)
                push(ArithToken.Word(operatorGlyph(node.op), gap = true, breakable = wrap, operator = node.op != '%')); gap = true
                emit(node.right, bracket = node.op == '/' || node.op == '%')
            }
        }
    }
    emit(root)
    return out
}

// Superscripts step down like TeX's script styles, never below a readable floor.
private const val ScriptScale = .72f
private const val ScriptFloor = .56f
// Exponent baseline sits this far above the base baseline, in base ems; a script's own power climbs further to clear it.
private const val RaiseEm = .42f
private const val ScriptRaiseEm = .7f
// Binary operator spacing, close to TeX's medium math space.
private const val GapEm = .24f

// Text serifs draw × and − at text size; the sans cut sizes them to the figures, as maths fonts do.
@Composable
internal fun operatorFamily() = FontFamily(Font(Res.font.google_sans_flex))

@Composable
internal fun ArithmeticLine(tokens: List<ArithToken>, style: TextStyle, modifier: Modifier = Modifier, root: TextStyle = style, onOverflow: (() -> Unit)? = null) {
    val operators = style.copy(fontFamily = operatorFamily())
    Layout(content = {
        tokens.forEach { token ->
            when (token) {
                is ArithToken.Word -> Text(token.text, style = if (token.operator) operators else style, softWrap = false, maxLines = 1)
                is ArithToken.Raise -> ArithmeticRaise(token, style, root)
            }
        }
    }, modifier = modifier) { measurables, constraints ->
        val gap = style.fontSize.toPx() * GapEm
        val items = measurables.map { it.measure(Constraints()) }
        val limit = if (constraints.hasBoundedWidth) constraints.maxWidth else Int.MAX_VALUE
        // Greedy fill by words: a word runs from one breakable token to the next.
        val lines = mutableListOf(mutableListOf<Int>())
        var x = 0f
        var i = 0
        while (i < items.size) {
            var end = i + 1
            while (end < items.size && !tokens[end].breakable) end++
            fun span(fromLineStart: Boolean) = (i until end).fold(0f) { w, k -> w + items[k].width + if (tokens[k].gap && !(fromLineStart && k == i)) gap else 0f }
            if (x > 0f && tokens[i].breakable && x + span(false) > limit) { lines += mutableListOf<Int>(); x = 0f }
            x += span(x == 0f)
            lines.last() += (i until end)
            i = end
        }
        fun ascent(p: Placeable) = p[FirstBaseline].takeIf { it != AlignmentLine.Unspecified } ?: p.height
        val rows = lines.map { line -> line to (line.maxOfOrNull { ascent(items[it]) } ?: 0) }
        val heights = rows.map { (line, a) -> a + (line.maxOfOrNull { items[it].height - ascent(items[it]) } ?: 0) }
        val widths = rows.map { (line, _) -> line.foldIndexed(0f) { n, w, k -> w + items[k].width + if (n > 0 && tokens[k].gap) gap else 0f } }
        if ((widths.maxOrNull() ?: 0f) > limit + .5f) onOverflow?.invoke()
        val width = (widths.maxOrNull() ?: 0f).roundToInt().coerceIn(constraints.minWidth, max(constraints.minWidth, limit))
        // Wrapped lines keep at least the style's leading between baselines.
        val leading = if (style.lineHeight.isSp) style.lineHeight.toPx().roundToInt() else 0
        val baselines = mutableListOf<Int>()
        var bottom = 0
        rows.forEachIndexed { r, (line, a) ->
            val baseline = max(bottom + a, (baselines.lastOrNull() ?: Int.MIN_VALUE / 2) + leading)
            baselines += baseline
            bottom = baseline + heights[r] - a
        }
        val height = bottom.coerceAtLeast(constraints.minHeight)
        layout(width, height, mapOf(FirstBaseline to (baselines.firstOrNull() ?: 0), LastBaseline to (baselines.lastOrNull() ?: 0))) {
            rows.forEachIndexed { r, (line, _) ->
                var at = 0f
                line.forEachIndexed { n, k ->
                    if (n > 0 && tokens[k].gap) at += gap
                    items[k].place(at.roundToInt(), baselines[r] - ascent(items[k]))
                    at += items[k].width
                }
            }
        }
    }
}

@Composable
private fun ArithmeticRaise(token: ArithToken.Raise, style: TextStyle, root: TextStyle) {
    val scale = max(ScriptScale, ScriptFloor * root.fontSize.value / style.fontSize.value).coerceAtMost(1f)
    val small = style.copy(fontSize = style.fontSize * scale, lineHeight = if (style.lineHeight.isSp) style.lineHeight * scale else style.lineHeight)
    Layout(content = {
        ArithmeticLine(token.base, style, root = root)
        ArithmeticLine(token.exponent, small, root = root)
    }) { (b, e), _ ->
        val base = b.measure(Constraints())
        val exponent = e.measure(Constraints())
        val em = style.fontSize.toPx()
        val baseline = base[FirstBaseline]
        val raise = if (style.fontSize.value < root.fontSize.value) ScriptRaiseEm else RaiseEm
        val expTop = baseline - (em * raise).roundToInt() - exponent[FirstBaseline]
        val shift = max(0, -expTop)
        val kern = (em * .04f).roundToInt()
        val height = max(base.height, expTop + exponent.height) + shift
        layout(base.width + kern + exponent.width, height, mapOf(FirstBaseline to baseline + shift, LastBaseline to baseline + shift)) {
            base.place(0, shift)
            exponent.place(base.width + kern, expTop + shift)
        }
    }
}

private val Superscripts = mapOf('0' to '⁰', '1' to '¹', '2' to '²', '3' to '³', '4' to '⁴', '5' to '⁵', '6' to '⁶', '7' to '⁷', '8' to '⁸', '9' to '⁹', '−' to '⁻', '+' to '⁺', '(' to '⁽', ')' to '⁾', ',' to ',', '.' to '·')

// The expression on one plain line, as a reply quote shows it: typeset operators, and exponents raised with superscript digits where they exist.
internal fun arithPlain(source: String): String? = parseArith(source)?.let { tree ->
    fun line(tokens: List<ArithToken>): String = buildString {
        tokens.forEach { t ->
            if (t.gap && isNotEmpty()) append(' ')
            when (t) {
                is ArithToken.Word -> append(t.text)
                is ArithToken.Raise -> {
                    append(line(t.base))
                    val power = line(t.exponent).filter { it != ' ' }
                    if (power.all { it in Superscripts }) power.forEach { append(Superscripts.getValue(it)) } else append("^(").append(power).append(')')
                }
            }
        }
    }
    line(arithTokens(tree, wrap = false))
}
