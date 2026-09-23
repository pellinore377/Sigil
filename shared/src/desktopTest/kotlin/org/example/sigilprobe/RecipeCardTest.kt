package org.sigil

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertEquals

class RecipeCardTest {
    @get:Rule val ui=createComposeRule()
    private val recipe=RecipeContent(RichText("Synthetic stew"),2,2,5400L,(1..7).map {RichText("$it cups stock")},List(7) {false},(1..4).map {RichText("Step text $it")})
    private val message=ChatMessage("card","author","",false,"9:41","read",false,emptyList(),emptyList(),null,true,peer="conversation",parts=listOf(MessagePart("r","recipe","",recipe=recipe)))

    @Test fun times_and_servings_read_for_people() {
        assertEquals("1 hr 30 min",recipeTime(5400)); assertEquals("20 min",recipeTime(1200)); assertEquals("2 hr",recipeTime(7200)); assertEquals("45 sec",recipeTime(45))
        assertEquals("1 serving",recipeServings(1)); assertEquals("4 servings",recipeServings(4))
    }

    @Test fun ingredients_check_off_in_place_and_the_rest_unfolds() {
        ui.setContent {MaterialTheme {Box(Modifier.width(400.dp)) {MessageCards(message,{""},null)}}}
        ui.onNode(hasContentDescription("Recipe. Synthetic stew")).assertExists()
        ui.onNodeWithText("2 servings · 1 hr 30 min").assertIsDisplayed()
        ui.onNode(hasText("1 cups stock") and hasClickAction()).performClick().assertIsOn()
        ui.onNodeWithText("6 cups stock").assertDoesNotExist()
        ui.onNodeWithText("Step text 4").assertDoesNotExist()
        ui.onNodeWithText("Show 2 more ingredients and 1 more step").performClick()
        ui.onNodeWithText("7 cups stock").assertIsDisplayed()
        ui.onNodeWithText("Step text 4").assertIsDisplayed()
        ui.onNodeWithText("Show less").assertIsDisplayed()
    }

    @Test fun servings_scale_from_the_card_and_mark_adjusted_amounts() {
        var asked=0
        ui.setContent {MaterialTheme {CompositionLocalProvider(LocalRecipeScale provides {_,_,serves->asked=serves
            recipe.copy(serves=serves,ingredients=listOf(RichText("1½ cups stock"))+recipe.ingredients.drop(1),scaled=listOf(true)+List(6) {false})}) {
            Box(Modifier.width(400.dp)) {MessageCards(message,{""},null)}}}}
        ui.onNodeWithContentDescription("More servings").performClick()
        ui.waitUntil {asked==3}
        ui.onNodeWithText("3 servings").assertIsDisplayed()
        ui.onNodeWithText("1½ cups stock").assertIsDisplayed()
        ui.onNodeWithText("Amounts adjusted for 3 servings").assertIsDisplayed()
        ui.onAllNodesWithText("Adjusted").assertCountEquals(0)
        ui.onAllNodesWithText("Not adjusted",substring=true).assertCountEquals(4)
    }

    @Test fun only_quantities_the_scaler_left_alone_are_flagged() {
        assertEquals(true,recipeAsWritten(RichText("1-2 eggs"),false))
        assertEquals(false,recipeAsWritten(RichText("Salt to taste"),false))
        assertEquals(false,recipeAsWritten(RichText("400 g pasta"),true))
    }

    @Test fun sessions_restore_from_saved_state_and_stay_bounded() {
        val (first,fresh)=recipeSession("test/restore/0")
        assertEquals(true,fresh)
        first.checked=setOf(0,3); first.serves=4
        val saved=first.encode()
        val restored=RecipeSession().apply {restore(saved)}
        assertEquals(setOf(0,3),restored.checked); assertEquals(4,restored.serves)
        repeat(25) {recipeSession("test/fill/$it")}
        assertEquals(true,recipeSession("test/restore/0").second)
    }
}
