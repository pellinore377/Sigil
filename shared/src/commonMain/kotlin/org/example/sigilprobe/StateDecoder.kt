package org.sigil

import kotlinx.serialization.json.*

internal fun JsonObject.string(key:String)=this[key]?.jsonPrimitive?.contentOrNull.orEmpty()
internal fun JsonObject.bool(key:String,default:Boolean=false)=this[key]?.jsonPrimitive?.booleanOrNull ?: default
internal fun JsonObject.long(key:String)=this[key]?.jsonPrimitive?.longOrNull ?: 0L
internal fun JsonObject.objects(key:String)=this[key]?.takeUnless{it==JsonNull}?.jsonArray?.map {it.jsonObject}.orEmpty()
internal fun JsonObject.strings(key:String)=this[key]?.takeUnless{it==JsonNull}?.jsonArray?.map {it.jsonPrimitive.content}.orEmpty()
internal fun JsonObject.settings(key:String)=this[key]?.takeUnless{it==JsonNull}?.jsonObject?.mapValues {it.value.jsonPrimitive.content}.orEmpty()
internal fun JsonObject.optional(key:String)=this[key]?.jsonPrimitive?.contentOrNull

internal object StateDecoder {
    fun state(value:JsonObject,prior:MessengerState,clock:(Long)->String):MessengerState {
        val phase=value.string("phase")
        if(phase!="connected")return prior.copy(phase=phase,loginAddress=value.optional("server")?:prior.loginAddress)
        val chats=value.objects("chats").map {c->ChatSummary(c.string("id"),c.string("address"),c.string("preview"),clock(c.long("timestamp")),c.bool("verified"),
            c.objects("devices").map {ChatDevice(it.string("id"),it.string("fingerprint"),it.bool("identity_verified"),it.bool("blocked"),it.bool("changed"))},
            displayName=c.string("name"),unread=c.long("unread").toInt(),pinned=c.bool("pinned"),snoozed=c.bool("snoozed"),hidden=c.bool("hidden"),presence=c.optional("presence")?:"inactive",collections=c.strings("collections"),typing=c.strings("typing"),draft=c.string("draft"),group=c.bool("group"),avatar=c.string("avatar"),ui=c.settings("ui"),contactOnly=c.bool("contact_only"),readReceipts=c.bool("read_receipts",true),typingIndicators=c.bool("typing_indicators",true),presenceSharing=c.bool("presence_sharing"),request=c.optional("request")?:"none",identityReview=c.optional("identity_review"))}
        return prior.copy(phase=phase,address=value.string("address"),device=value.string("device"),fingerprint=value.string("fingerprint"),chats=chats,ui=value.settings("ui"),profileAvatar=value.string("profile_avatar"),photoPending=value.bool("photo_pending"),collectionsEnabled=value.bool("collections_enabled"),collections=value.objects("collections").map {CollectionItem(it.string("id"),it.string("name"),it.optional("icon")?:"folder")},readReceipts=value.bool("read_receipts"),typingIndicators=value.bool("typing_indicators"),presenceSharing=value.bool("presence_sharing"),invitations=value.objects("invitations").map {GroupInvitation(it.string("id"),it.string("peer"),it.string("group"))})
    }
fun devices(value:JsonObject,prior:MessengerState,append:Boolean):MessengerState {
    val devices=(if(append)prior.devices else emptyList()).associateBy{it.id}.toMutableMap()
    value.objects("devices").forEach {v->val old=devices[v.string("id")];devices[v.string("id")]=AccountDevice(v.string("id"),v.bool("current"),v.optional("label")?:old?.label,v["revoked"]?.jsonPrimitive?.booleanOrNull?:old?.revoked,v["expires"]?.jsonPrimitive?.longOrNull?:old?.expires,v.optional("fingerprint")?:old?.fingerprint,if(v.optional("fingerprint")==null)old?.verified==true else v.bool("verified"))}
    return prior.copy(devices=devices.values.sortedByDescending{it.current},devicesNext=value.optional("next"))
}
fun search(value:JsonObject,clock:(Long)->String)=value.objects("hits").map {h->SearchHit(h.string("peer"),h.string("id"),h.string("author"),h.string("text"),clock(h.long("timestamp")),h.bool("pinned"),h.bool("noted"),h.string("kind"),h.bool("thread"),h.optional("thread_author")?.let {a->h.optional("thread_message")?.let {ThreadTarget(a,it)}})}
    fun messages(value:JsonObject,peer:String,clock:(Long)->String,separator:(Long)->String=clock):List<ChatMessage> = value.objects("messages").map {m->
        ChatMessage(m.string("id"),m.string("author"),m.string("text"),m.bool("mine"),clock(m.long("timestamp")),m.string("delivery"),m.bool("pinned"),m.strings("reactions"),m.strings("my_reactions"),m.optional("reply"),m.bool("read_by_me"),
            timestamp=m.long("timestamp"),separator=separator(m.long("timestamp")),readers=m.strings("readers"),noted=m.bool("noted"),threadAuthor=m.optional("thread_author"),threadMessage=m.optional("thread_message"),editable=m.bool("editable",true),kind=m.optional("kind")?:"Text",peer=peer,
            attachment=m["attachment"]?.takeUnless{it==JsonNull}?.jsonObject?.let {AttachmentDetails(it.string("name"),it.string("media_type"),it.long("length"),it.string("caption"))},
            parts=m.objects("parts").map {ContentDecoder.part(it.toString(),clock)},threadPreview=m.optional("thread_preview"),replyAuthor=m.optional("reply_author"),replyMine=m.bool("reply_mine"),replyMessage=m.optional("reply_message"),
            replyAttachment=m["reply_attachment"]?.takeUnless{it==JsonNull}?.jsonObject?.let {AttachmentDetails(it.string("name"),it.string("media_type"),it.long("length"),it.string("caption"))})
    }
}
