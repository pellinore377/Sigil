@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class,org.jetbrains.compose.resources.ExperimentalResourceApi::class)
package org.sigil

import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.viewinterop.WebElementView
import kotlinx.browser.document
import org.w3c.dom.*
import org.w3c.dom.events.Event

private fun cameraFont(name:String)="/web/composeResources/sigil.shared.generated.resources/font/$name"

private class CameraDom(val video:HTMLVideoElement) {
    val root=(document.createElement("div") as HTMLElement).apply {setAttribute("style","position:relative;width:100%;height:100%;overflow:hidden;border-radius:24px;background:#000;color:#fff")}
    private fun node(tag:String,style:String)= (document.createElement(tag) as HTMLElement).apply {setAttribute("style",style);root.appendChild(this)}
    private val fonts = (document.createElement("style") as HTMLStyleElement).also { document.head?.appendChild(it) }
    val image=node("img","display:none;position:absolute;inset:0;width:100%;height:100%;object-fit:contain")
    init {
        video.setAttribute("style","display:block;position:absolute;inset:0;width:100%;height:100%;object-fit:cover")
        root.insertBefore(video,image)
        fonts.textContent="@font-face{font-family:SigilCameraSymbols;src:url('${cameraFont("material_symbols.ttf")}')}@font-face{font-family:SigilCameraSerif;src:url('${cameraFont("newsreader.ttf")}')}@font-face{font-family:SigilCameraSans;src:url('${cameraFont("google_sans_flex.ttf")}')}"
        node("div","pointer-events:none;position:absolute;inset:0 0 auto;height:88px;background:linear-gradient(#0008,transparent)")
        node("div","pointer-events:none;position:absolute;inset:auto 0 0;height:136px;background:linear-gradient(transparent,#000c)")
    }
    private val handlers=mutableListOf<Pair<HTMLElement,(Event)->Unit>>()
    fun button(icon:String,label:String,position:String)= (node("button","position:absolute;$position;width:48px;height:48px;border:0;border-radius:50%;background:transparent;color:white;font:28px SigilCameraSymbols;cursor:pointer;padding:0") as HTMLButtonElement).apply {type="button";textContent=icon;setAttribute("aria-label",label)}
    val close=button("close","Back to attachments","top:8px;left:8px")
    val flip=button("flip_camera_android","Switch camera","bottom:28px;right:16px")
    val shutter=button("","Take photo","bottom:16px;left:50%;transform:translateX(-50%);").apply {style.width="72px";style.height="72px";style.border="3px solid white"}
    val shutterFace=(document.createElement("span") as HTMLElement).apply {setAttribute("style","display:block;width:56px;height:56px;background:white;border-radius:50%;margin:auto");shutter.appendChild(this)}
    val retake=button("refresh","Retake","bottom:20px;left:50%;transform:translateX(-50%)")
    val retry=button("refresh","Retry camera","top:calc(50% + 48px);left:50%;transform:translateX(-50%)")
    val issue=node("div","position:absolute;top:50%;left:24px;right:24px;transform:translateY(-50%);padding:12px;border-radius:12px;background:#000b;font:14px system-ui;text-align:center")
    fun bind(node:HTMLElement,action:()->Unit){val listener:(Event)->Unit={action()};node.addEventListener("click",listener);handlers+=node to listener}
    fun dispose(){handlers.forEach {(node,listener)->node.removeEventListener("click",listener)};handlers.clear();fonts.remove();root.remove();image.removeAttribute("src")}
}

@Composable internal fun WebCameraView(video:HTMLVideoElement,url:String?,ready:Boolean,busy:Boolean,issue:String?,modifier:Modifier,close:()->Unit,capture:()->Unit,retake:()->Unit,flip:()->Unit,retry:()->Unit) {
    val appearance=LocalAppearance.current
    val actions by rememberUpdatedState(listOf(close,capture,retake,flip,retry))
    val view=remember(video){CameraDom(video).also {v->listOf(v.close,v.shutter,v.retake,v.flip,v.retry).forEachIndexed {i,button->v.bind(button){actions[i]()}}}}
    SideEffect {
        val photo=url!=null
        view.video.style.display=if(photo)"none" else "block"
        view.image.style.display=if(photo)"block" else "none"
        if(url!=null && view.image.getAttribute("src")!=url)view.image.setAttribute("src",url)
        if(url==null)view.image.removeAttribute("src")
        view.shutter.style.display=if(photo)"none" else "block";view.shutter.disabled=!ready||busy
        view.shutter.style.opacity=if(ready&&!busy)"1" else ".4"
        view.retake.style.display=if(photo)"block" else "none";view.retake.disabled=busy
        view.flip.style.display=if(photo)"none" else "block";view.flip.disabled=busy
        view.issue.textContent=issue.orEmpty();view.issue.style.display=if(issue==null)"none" else "block"
        view.issue.style.fontFamily=if(appearance.font=="Newsreader")"SigilCameraSerif" else "SigilCameraSans";view.issue.style.fontSize="${14*appearance.textScale}px"
        view.retry.style.display=if(issue!=null&&!photo)"block" else "none";view.retry.disabled=busy
    }
    DisposableEffect(view){onDispose{view.dispose()}}
    WebElementView(factory={view.root},modifier=modifier)
}
