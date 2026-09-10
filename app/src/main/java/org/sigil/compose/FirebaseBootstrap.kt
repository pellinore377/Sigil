package org.sigil.compose

import android.app.Application
import android.content.Context
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONObject

internal object FirebaseBootstrap {
    private val lock=Any()
    private fun preferences(context:Context)=context.getSharedPreferences("firebase_bootstrap",0)
    private fun options(source:String):FirebaseOptions {
        require(source.length<=1024)
        val json=JSONObject(source)
        require(json.keys().asSequence().toSet()==setOf("project_id","application_id","api_key","sender_id"))
        return FirebaseOptions.Builder().setProjectId(json.getString("project_id"))
            .setApplicationId(json.getString("application_id")).setApiKey(json.getString("api_key"))
            .setGcmSenderId(json.getString("sender_id")).build()
    }
    private fun app(context:Context)=FirebaseApp.getApps(context).firstOrNull {it.name==FirebaseApp.DEFAULT_APP_NAME}
    private fun desired(context:Context)=preferences(context).getString("options",null)?.let(::options)
    fun start(context:Context)=synchronized(lock) {
        if(NativeSignOut.pending(context) || !preferences(context).getBoolean("enabled",false))return@synchronized
        val wanted=desired(context) ?: return@synchronized
        if(app(context)==null)FirebaseApp.initializeApp(context,wanted).setDataCollectionDefaultEnabled(false)
    }
    fun restore(context:Context) {
        if(!preferences(context).getBoolean("enabled",false))return
        val state=NativePush.execute(context,"status")
        if(state.getBoolean("configured") && state.optString("choice")!="fcm")stop(context) else start(context)
    }
    fun prepare(context:Context,source:String):Boolean=synchronized(lock) {
        val wanted=options(source)
        check(!NativeSignOut.pending(context))
        check(preferences(context).edit().putString("options",source).putBoolean("enabled",true).commit())
        val current=app(context)
        if(current!=null && current.options!=wanted) {
            FirebaseMessaging.getInstance().isAutoInitEnabled=false
            return@synchronized false
        }
        start(context)
        true
    }
    fun current(context:Context):Boolean=synchronized(lock) {
        !NativeSignOut.pending(context) && preferences(context).getBoolean("enabled",false) &&
            app(context)?.options?.let {it==desired(context)}==true
    }
    fun <T> ifCurrent(context:Context,action:()->T):T?=synchronized(lock) {
        if(current(context))action() else null
    }
    fun stop(context:Context)=synchronized(lock) {
        check(preferences(context).edit().putBoolean("enabled",false).commit())
        if(app(context)!=null)FirebaseMessaging.getInstance().isAutoInitEnabled=false
    }
}

class SigilApplication:Application() {
    override fun onCreate() {
        super.onCreate()
        if(android.os.Process.myUid()!=applicationInfo.uid)return
        try {FirebaseBootstrap.restore(this)} catch(_:Exception) { }
    }
}
