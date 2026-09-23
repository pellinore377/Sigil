package org.sigil

// Rust sets display to the built-in category name, or "Choice" for a custom list.
private val categoryNames = mapOf("yesno" to "Yes or no")

internal fun choiceDescription(value: UtilityContent): String {
    val result = value.motion?.result?.ifEmpty { null } ?: value.rich?.text ?: ""
    val category = value.display.takeIf { it.isNotBlank() && it != "Choice" } ?: return "Card pick. $result"
    val name = categoryNames[category] ?: category.replaceFirstChar { it.uppercase() }
    return "Category pick. $name: $result"
}
