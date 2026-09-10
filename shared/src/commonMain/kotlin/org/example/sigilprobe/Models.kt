package org.sigil

data class ChatDevice(val id: String, val fingerprint: String, val verified: Boolean, val blocked: Boolean, val changed: Boolean)
data class ChatSummary(val id: String, val address: String, val preview: String, val time: String, val verified: Boolean, val devices: List<ChatDevice>,
    val displayName: String = "", val unread: Int = 0, val pinned: Boolean = false, val snoozed: Boolean = false,
    val hidden: Boolean = false, val presence: String = "inactive", val collections: List<String> = emptyList(),
    val typing: List<String> = emptyList(), val draft: String = "", val group: Boolean = false, val avatar: String = "", val ui: Map<String, String> = emptyMap(), val contactOnly: Boolean = false,
    val readReceipts: Boolean = true, val typingIndicators: Boolean = true, val presenceSharing: Boolean = false, val request: String = "none", val archived: Boolean = false, val identityReview: String? = null) {
    val name get() = displayName.ifEmpty { address.removePrefix("@").substringBefore(':') }
}
data class ChatMessage(val id: String, val author: String, val text: String, val mine: Boolean, val time: String,
    val delivery: String, val pinned: Boolean, val reactions: List<String>, val myReactions: List<String>, val reply: String?, val readByMe: Boolean,
    val timestamp: Long = 0, val separator: String = "", val readers: List<String> = emptyList(), val noted: Boolean = false,
    val threadAuthor: String? = null, val threadMessage: String? = null, val editable: Boolean = true, val kind: String = "Text", val peer: String = "", val attachment: AttachmentDetails? = null, val parts: List<MessagePart> = emptyList(), val threadPreview: String? = null)
data class MessagePart(val id: String, val kind: String, val text: String, val items: List<CardItem> = emptyList(), val multiple: Boolean = false, val closed: Boolean = false, val voters: Long? = null, val date: String = "", val latitude: Double = 0.0, val longitude: Double = 0.0, val rich: RichText? = null,
    val locationMode: String = "pin", val sampledAt: Long = 0, val accuracyCm: Long? = null, val until: Long? = null, val stopped: Boolean = false, val canStop: Boolean = false, val table: TableContent? = null, val recipe: RecipeContent? = null, val chart: ChartContent? = null)
data class TableContent(val columns: List<RichText>, val rows: List<List<RichText>>, val numericOrder: List<List<Int>?>, val copyRows: List<String?>, val copyTable: String?)
data class RecipeContent(val title: RichText, val serves: Int?, val originalServes: Int?, val seconds: Long?, val ingredients: List<RichText>, val scaled: List<Boolean>, val steps: List<RichText>)
data class ChartContent(val kind: String, val title: RichText, val horizontal: Boolean, val zero: Float, val yTicks: List<String>, val xTicks: List<String>, val copyData: String?, val points: List<ChartPoint>)
data class ChartPoint(val label: RichText, val x: Float, val y: Float, val value: String, val xValue: String?, val share: Float, val percent: String)
data class CardItem(val id: String, val text: String, val checked: Boolean, val enabled: Boolean, val count: Long? = null, val rich: RichText? = null)
data class RichText(val text: String, val spans: List<RichSpan> = emptyList(), val blocks: List<RichBlock> = emptyList(), val codeTokens: List<CodeToken> = emptyList())
data class RichSpan(val start: Int, val end: Int, val flags: Set<String> = emptySet(), val colors: List<String> = emptyList(), val size: Int = 0, val reveal: String = "", val link: String? = null)
data class RichBlock(val start: Int, val end: Int, val kind: String, val level: Int = 0, val language: String = "")
data class CodeToken(val start: Int, val end: Int, val role: String)
data class AttachmentDetails(val name: String, val mediaType: String, val bytes: Long, val caption: String = "", val draft: Boolean = false)
data class CollectionItem(val id: String, val name: String, val icon: String = "folder")
data class GroupInvitation(val id: String, val peer: String, val group: String)
data class Transfer(val request: String, val peer: String, val name: String, val bytes: Long, val phase: String, val draft: Boolean = false, val mediaType: String = "application/octet-stream")
data class VoiceState(val phase: String = "Idle", val peer: String = "", val seconds: Long = 0, val levels: List<Float> = emptyList(), val playing: Boolean = false,
    val paused: Boolean = false, val position: Long = 0, val duration: Long = 0)
data class CallParticipant(val id: String, val peer: String, val name: String, val own: Boolean, val verified: Boolean, val audio: Boolean, val camera: Boolean, val screen: Boolean, val fingerprint: String = "")
data class CallSummary(val id: String, val phase: String, val direct: Boolean, val created: Long, val participants: List<CallParticipant>, val canInvite: Boolean = false, val name: String = "", val outgoing: Boolean = false, val time: String = "")
data class ActiveCall(val call: CallSummary, val name: String, val connection: String = "connecting", val seconds: Long = 0, val muted: Boolean = false, val speaker: Boolean = false, val camera: Boolean = false, val screen: Boolean = false, val levels: Map<String, Float> = emptyMap())
data class ThreadTarget(val author: String, val id: String)
data class AccountDevice(val id: String, val current: Boolean, val label: String? = null, val revoked: Boolean? = null, val expires: Long? = null, val fingerprint: String? = null, val verified: Boolean = false)
data class StorageDetails(val database: Long, val media: Long, val mediaUsed: Long, val budget: Long, val recovery: Boolean, val checkpoint: String?, val unprotected: Long, val historyDays: Int? = null, val restoring: Boolean = false)
data class NotificationSettings(val enabled: Boolean, val messages: Boolean, val calls: Boolean)
data class PushDistributor(val id: String, val name: String)
data class PushSettings(val enabled: Boolean, val status: String, val distributor: String?, val distributors: List<PushDistributor>)
data class AccountAccess(val configuration: Long, val transition: Long, val issuer: String?, val linked: Boolean, val retiring: Boolean, val acknowledged: Boolean, val linkPending: Boolean)
data class SearchHit(val peer: String, val id: String, val author: String, val text: String, val time: String,
    val pinned: Boolean = false, val noted: Boolean = false, val kind: String = "Text", val thread: Boolean = false, val threadTarget: ThreadTarget? = null)
data class MessengerState(val phase: String = "loading", val address: String = "", val fingerprint: String = "", val device: String = "",
    val chats: List<ChatSummary> = emptyList(), val selected: String? = null, val messages: List<ChatMessage> = emptyList(),
    val more: Boolean = false, val busy: Boolean = false, val issue: String? = null, val sent: Long = 0, val sentText: String? = null,
    val loginAddress: String = "", val loginMethods: LoginMethods? = null, val discovering: Boolean = false, val discoveryIssue: String? = null,
    val collectionsEnabled: Boolean = false, val collections: List<CollectionItem> = emptyList(),
    val searchHits: List<SearchHit> = emptyList(), val searching: Boolean = false, val searchQuery: String = "",
    val typing: List<String> = emptyList(), val ui: Map<String, String> = emptyMap(),
    val profileName: String = "", val profileRevision: Long? = null, val profileAvatar: String = "", val photoPending: Boolean = false,
    val readReceipts: Boolean = true, val typingIndicators: Boolean = true, val presenceSharing: Boolean = false,
    val invitations: List<GroupInvitation> = emptyList(), val transfers: List<Transfer> = emptyList(), val voice: VoiceState = VoiceState(), val searchMore: Boolean = false, val historical: Boolean = false,
    val calls: List<CallSummary> = emptyList(), val call: ActiveCall? = null, val threadTarget: ThreadTarget? = null, val people: Map<String, String> = emptyMap(),
    val allowRequests: Boolean? = null, val devices: List<AccountDevice> = emptyList(), val devicesNext: String? = null, val storage: StorageDetails? = null, val notifications: NotificationSettings? = null, val accountAccess: AccountAccess? = null, val push: PushSettings? = null)
data class LoginMethods(val server: String, val sso: Boolean, val password: Boolean, val invitation: Boolean)
