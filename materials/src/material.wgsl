struct Face {plane:vec4f, right:vec4f, up:vec4f}
struct Obstacle {center:vec4f, half_size:vec4f}
struct Settings {
    viewport: vec4f,
    body: vec4f,
    secondary: vec4f,
    ink: vec4f,
    material: vec4f,
    details: vec4f,
    rotation: vec4f,
    position: vec4f,
    lighting: vec4f,
    options: vec4f,
    backdrop: vec4f,
    faces: array<Face,30>,
    labels: array<vec4f,30>,
    obstacles: array<Obstacle,7>,
}
@group(0) @binding(0) var<uniform> u: Settings;
@group(0) @binding(1) var marks: texture_2d<f32>;
@group(0) @binding(2) var label: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var orbit: texture_2d<f32>;
@group(0) @binding(5) var digits: texture_2d<f32>;
const PI: f32 = 3.14159265;
fn qrot(q: vec4f, v: vec3f) -> vec3f { return v + 2.0 * cross(q.xyz, cross(q.xyz, v) + q.w * v); }
fn local(p: vec3f) -> vec3f { return qrot(vec4f(-u.rotation.xyz,u.rotation.w), p - u.position.xyz)/u.position.w; }
fn world_dir(p: vec3f) -> vec3f { return qrot(u.rotation,p); }
fn rounded_box(p: vec3f,b: vec3f,r: f32)->f32 {
    let d=abs(p)-b+vec3f(r);return length(max(d,vec3f(0)))+min(max(d.x,max(d.y,d.z)),0.0)-r;
}
fn rounded_rect(p:vec2f,b:vec2f,r:f32)->f32 {
    let d=abs(p)-b+r;return length(max(d,vec2f(0)))+min(max(d.x,d.y),0.0)-r;
}
fn hash(p:vec3f)->f32 {return fract(sin(dot(p,vec3f(127.1,311.7,74.7)))*43758.5453);}
fn noise(p:vec3f)->f32 {
    let i=floor(p);let f=fract(p);let t=f*f*(3.0-2.0*f);
    return mix(mix(mix(hash(i),hash(i+vec3f(1,0,0)),t.x),mix(hash(i+vec3f(0,1,0)),hash(i+vec3f(1,1,0)),t.x),t.y),mix(mix(hash(i+vec3f(0,0,1)),hash(i+vec3f(1,0,1)),t.x),mix(hash(i+vec3f(0,1,1)),hash(i+vec3f(1,1,1)),t.x),t.y),t.z);
}
fn cloud(p:vec3f)->f32 {return noise(p*2.6+vec3f(noise(p*4.0)*1.5))*.65+noise(p*6.0)*.35;}
fn logo(uv:vec2f)->f32 {return textureSampleLevel(marks,linear_sampler,clamp(uv,vec2f(0),vec2f(1)),0).r;}
fn glyph_mask(uv:vec2f,glyph:u32)->f32 {
    if any(uv<vec2f(0))||any(uv>vec2f(1)){return 0.0;}
    return textureSampleLevel(digits,linear_sampler,(vec2f(f32(glyph%7u),f32(glyph/7u))*73.0+clamp(uv,vec2f(.01),vec2f(.99))*73.0)/512.0,0).r;
}
fn number_mask(p:vec3f,face:u32)->f32 {
    if u.options.x==4. {
        let f=u.faces[face];let center=f.plane.xyz*f.plane.w;var mask=0.;
        for(var j=0u;j<4u;j++){if j!=face {
            let vertex=-u.faces[j].plane.xyz*u.faces[j].plane.w*3.;let up=normalize(vertex-center);let right=cross(up,f.plane.xyz);
            let offset=p-mix(center,vertex,.48);let uv=vec2f(dot(offset,right),-dot(offset,up))/(u.labels[face].z*.48)+.5;
            mask=max(mask,glyph_mask(uv,j));
        }}
        return mask;
    }
    if u.options.y<.5 {
        let f=u.faces[face];let uv=vec2f(dot(p,f.right.xyz),dot(p,f.up.xyz));let value=face+1u;var d=10.0;
        if value==1u||value==3u||value==5u{d=length(uv);}
        if value>=2u{d=min(d,min(length(uv-vec2f(-.28,.28)),length(uv-vec2f(.28,-.28))));}
        if value>=4u{d=min(d,min(length(uv-vec2f(.28,.28)),length(uv-vec2f(-.28,-.28))));}
        if value==6u{d=min(d,min(length(uv-vec2f(-.28,0)),length(uv-vec2f(.28,0))));}
        return 1.0-smoothstep(.067,.084,d);
    }
    let f=u.faces[face];let bounds=u.labels[face];let uv=vec2f(dot(p,f.right.xyz)-bounds.x,-dot(p,f.up.xyz)+bounds.y)/select(bounds.z,bounds.w,u.options.y>1.5)+.5;
    if any(uv<vec2f(0))||any(uv>vec2f(1)){return 0.0;}
    var glyph=face;
    if u.options.y>1.5 && u.options.y<2.5 {glyph=select(30u+face,40u,face==0u);}
    if u.options.y>2.5 {glyph=select(face-1u,30u,face==0u);}
    return glyph_mask(uv,glyph);
}
fn pip_mask(p:vec3f)->f32 {
    var face=0u;var distance=-100.0;
    for(var i=0u;i<u32(u.options.x);i++){let f=u.faces[i];let d=dot(p,f.plane.xyz)-f.plane.w;if d>distance{distance=d;face=i;}}
    return number_mask(p,face);
}
fn coin_mark(p:vec3f)->f32 {
    let r=length(p.xy);let ring=(1.0-smoothstep(.010,.018,abs(r-.82)));
    var emblem=logo(p.xy*vec2f(.64,-.64)+.5);
    if p.z<0 {
        emblem=textureSampleLevel(orbit,linear_sampler,clamp(p.xy*vec2f(-.64,-.64)+.5,vec2f(0),vec2f(1)),0).r;
    }
    return max(ring,emblem)* (1.0-smoothstep(.90,.92,r));
}
fn shape(p:vec3f)->f32 {
    let kind=i32(u.viewport.z);
    if kind==0 {
        var d=-100.0;var nearest=-100.0;var face=0u;
        for(var i=0u;i<u32(u.options.x);i++){
            let f=u.faces[i];let b=dot(p,f.plane.xyz)-f.plane.w;if b>nearest{nearest=b;face=i;}
            let k=min(u.details.x,.10);let h=max(k-abs(d-b),0.0)/k;
            d=max(d,b)+h*h*k*.25;
        }
        if abs(d)<.025 {return d+number_mask(p,face)*u.details.y*.012;}
        return d;
    }
    if kind==1 {
        let d=vec2f(rounded_rect(p.xy,vec2f(.72,1.04),.10),abs(p.z)-.022);
        return min(max(d.x,d.y),0.0)+length(max(d,vec2f(0)))-.005;
    }
    let r=length(p.xy);let edge=.017*(.5+.5*cos(atan2(p.y,p.x)*100.0))*(1.0-smoothstep(.05,.08,abs(p.z)));
    let d=vec2f(r-(.96-edge),abs(p.z)-.092);
    let base=min(max(d.x,d.y),0.0)+length(max(d,vec2f(0)))-.012;
    let rim=-.016*(1.0-smoothstep(.01,.035,abs(r-.90)));
    return base+(coin_mark(p)*u.details.y*.014+rim)*smoothstep(.06,.08,abs(p.z));
}
fn normal(p:vec3f)->vec3f {
    let e=select(.0015,max(.0015,8.0/u.viewport.y/u.position.w),u.options.w>.5);
    if i32(u.viewport.z)==0 {
        var d=-100.0;var gradient=vec3f(0);var nearest=-100.0;var face=0u;let k=min(u.details.x,.10);
        for(var i=0u;i<u32(u.options.x);i++) {
            let f=u.faces[i].plane;let b=dot(p,f.xyz)-f.w;
            if b>nearest{nearest=b;face=i;}
            let weight=clamp(.5+.5*(d-b)/k,0.0,1.0);
            gradient=mix(f.xyz,gradient,weight);
            let h=max(k-abs(d-b),0.0)/k;d=max(d,b)+h*h*k*.25;
        }
        let bump=vec3f(number_mask(p+vec3f(e,0,0),face)-number_mask(p-vec3f(e,0,0),face),number_mask(p+vec3f(0,e,0),face)-number_mask(p-vec3f(0,e,0),face),number_mask(p+vec3f(0,0,e),face)-number_mask(p-vec3f(0,0,e),face));
        return normalize(gradient+bump*u.details.y*.012/(2.0*e));
    }
    return normalize(vec3f(shape(p+vec3f(e,0,0))-shape(p-vec3f(e,0,0)),shape(p+vec3f(0,e,0))-shape(p-vec3f(0,e,0)),shape(p+vec3f(0,0,e))-shape(p-vec3f(0,0,e))));
}
// Clip dice to their convex hull before evaluating bevels and engravings.
fn hull_interval(ro:vec3f,rd:vec3f)->vec2f {
    var near=0.0;var far=12.0/u.position.w;
    for(var i=0u;i<u32(u.options.x);i++) {
        let f=u.faces[i].plane;let distance=f.w-dot(ro,f.xyz);let denom=dot(rd,f.xyz);
        if abs(denom)<.000001 {if distance<0.0{return vec2f(1,-1);}continue;}
        let t=distance/denom;
        if denom<0.0 {near=max(near,t);}else{far=min(far,t);}
    }
    return vec2f(near,far);
}
fn trace(ro:vec3f,rd:vec3f)->f32 {

    var t=0.0;var end=12.0/u.position.w;if i32(u.viewport.z)==0{let interval=hull_interval(ro,rd);t=interval.x;end=interval.y;if t>end{return -1.0;}}
    for(var i=0;i<96;i++){let d=shape(ro+rd*t);if d<.0008{return t;}t+=d*.82;if t>end{break;}}
    return -1.0;
}
fn environment(d:vec3f,rough:f32)->vec3f {
    let a=u.lighting.x;
    let v=vec3f(cos(a)*d.x-sin(a)*d.z,d.y,sin(a)*d.x+cos(a)*d.z);
    let sky=mix(vec3f(.11,.13,.16),vec3f(.62,.67,.76),smoothstep(-.3,1.0,v.y));
    let key=pow(max(dot(v,normalize(vec3f(-.7,1.0,1.1))),0.0),mix(100.0,7.0,rough));
    let strip=pow(max(dot(v,normalize(vec3f(.9,.35,.7))),0.0),mix(260.0,12.0,rough));
    let rim=pow(max(dot(v,normalize(vec3f(-.4,.6,-1.0))),0.0),mix(80.0,5.0,rough));
    return sky+vec3f(1.0,.89,.72)*key*4.0+vec3f(.68,.83,1.0)*strip*3.0+vec3f(1.0,.9,.8)*rim;
}
fn floor_color(p:vec3f)->vec3f {
    let dark=u.viewport.w;
    let base=mix(vec3f(.79,.77,.73),vec3f(.031,.034,.039),dark);
    let shadow=exp(-dot(p.xz-u.position.xz,p.xz-u.position.xz)*1.5)*.55/(1.0+max(u.position.y,0.0));
    return base*(1.0-shadow)+vec3f(noise(p*180.0)*.002);
}
fn background(ro:vec3f,rd:vec3f)->vec3f {
    if rd.y<-.001 {let t=(-1.25-ro.y)/rd.y;if t>0.0{return floor_color(ro+rd*t);}}
    return mix(vec3f(.79,.77,.73),vec3f(.031,.034,.039),u.viewport.w);
}
fn obstacle_distance(p:vec3f)->f32 {
    var d=100.0;
    for(var i=0u;i<7u;i++){let b=u.obstacles[i];d=min(d,rounded_box(p-b.center.xyz,b.half_size.xyz,b.half_size.w));}
    return d;
}
fn obstacle_trace(ro:vec3f,rd:vec3f)->f32 {
    var t=0.0;
    for(var i=0;i<64;i++){let d=obstacle_distance(ro+rd*t);if d<.002{return t;}t+=d*.9;if t>12.0{break;}}
    return -1.0;
}
fn obstacle_color(p:vec3f)->vec3f {
    let base=mix(vec3f(.63),vec3f(.11,.12,.14),u.viewport.w);
    let ink=mix(vec3f(.12),vec3f(.65),u.viewport.w);
    for(var i=0u;i<3u;i++) {
        let b=u.obstacles[i];let q=p-b.center.xyz;
        if abs(q.x)<b.half_size.x && abs(q.z)<b.half_size.z {
            let uv=q.xz/3.0+.5;
            let text=textureSampleLevel(label,linear_sampler,uv,0).r;
            return mix(base,ink,text);
        }
    }
    let q=p.xz-vec2f(0,5);
    if abs(q.y)<.55 && abs(q.x)<4.5 {
        let field=1.0-smoothstep(-.015,.015,rounded_rect(q,vec2f(2.95,.35),.20));
        let plus=1.0-smoothstep(.015,.03,min(rounded_rect(q-vec2f(-3.65,0),vec2f(.19,.025),.01),rounded_rect(q-vec2f(-3.65,0),vec2f(.025,.19),.01)));
        let send=1.0-smoothstep(.018,.03,abs(length(q-vec2f(3.65,0))-.19));
        return mix(base*(1.0-field*.25),ink,max(plus,send));
    }
    return base;
}
fn border_line(distance:f32)->f32 {
    let aa=max(.002,1.4/u.viewport.y);
    return 1.0-smoothstep(.004,.004+aa,abs(distance));
}
fn card_border(p:vec2f)->f32 {
    let a=abs(p);let outer=border_line(rounded_rect(p,vec2f(.635,.955),.065));
    if u.options.z<.5{return outer;}
    let inner=border_line(rounded_rect(p,vec2f(.59,.91),.05));
    let c=a-vec2f(.49,.81);
    if u.options.z>1.5 {
        let diamond=border_line(abs(c.x)+abs(c.y)-.052);
        let rails=max(border_line(a.x-.612)*step(a.y,.69),border_line(a.y-.932)*step(a.x,.37));
        return max(max(outer,inner),max(diamond,rails));
    }
    let ring=border_line(length(c)-.055)*step(-.012,min(c.x,c.y));
    let leaf=border_line(length(c-vec2f(-.025,.025))-.035)*step(c.x,0.0)*step(0.0,c.y);
    let other=border_line(length(c-vec2f(.025,-.025))-.035)*step(0.0,c.x)*step(c.y,0.0);
    return max(max(outer,inner),max(ring,max(leaf,other)));
}
fn material_grain(p:vec3f)->f32 {
    let style=u.details.w;let kind=i32(u.viewport.z);
    if style<.5{return 0.;}
    if kind==0 {
        if style<1.5{return sin(p.x*17.+p.z*9.+noise(p*2.5)*11.)*.5;}
        if style<2.5{return sin(p.y*13.+p.x*5.+noise(p*3.)*8.)*.22+noise(p*4.)*.25;}
        return noise(p*32.)-.5;
    }
    if kind==2 {
        if style<1.5{return sin(p.y*145.+sin(p.x*9.)*.6)*.5;}
        if style<2.5{let cell=p.xy*13.+vec2f(noise(p*6.),noise(p*6.+vec3f(7.)))*1.4;let stagger=vec2f(fract(floor(cell.y)*.5),0);let q=fract(cell+stagger)-.5;return smoothstep(.12,.48,length(q))-.5;}
        return noise(p*7.)-.5;
    }
    if style<1.5{return sin(p.y*140.)*.38+sin(p.x*34.)*.12;}
    if style<2.5{return sin(p.x*100.)*sin(p.y*100.)*.5;}
    return noise(p*18.)-.5;
}
fn fresnel(c:f32,f0:vec3f)->vec3f {return f0+(1.0-f0)*pow(1.0-clamp(c,0.0,1.0),5.0);}
fn brdf(n:vec3f,v:vec3f,l:vec3f,base:vec3f,metal:f32,rough:f32)->vec3f {
    let h=normalize(v+l);let nv=max(dot(n,v),.001);let nl=max(dot(n,l),0.0);let nh=max(dot(n,h),0.0);
    let a=rough*rough;let a2=a*a;let den=nh*nh*(a2-1.0)+1.0;
    let distribution=a2/max(PI*den*den,.00001);let k=(rough+1.0)*(rough+1.0)/8.0;
    let geometry=nv/(nv*(1.0-k)+k)*nl/(nl*(1.0-k)+k);
    let f=fresnel(max(dot(h,v),0.0),mix(vec3f(.04),base,metal));
    return ((1.0-f)*(1.0-metal)*base/PI+distribution*geometry*f/max(4.0*nv*nl,.001))*nl;
}
fn surface_color(p:vec3f)->vec3f {
    let kind=i32(u.viewport.z);var base=u.body.rgb;
    if kind==0 {
        let grain=material_grain(p);base=mix(u.body.rgb,u.secondary.rgb,smoothstep(.25,.77,cloud(p)));
        if u.details.w>.5 && u.details.w<1.5 {let vein=smoothstep(.36,.49,grain);base=mix(u.body.rgb*.38,u.secondary.rgb*1.45+vec3f(.20),vein);}
        if u.details.w>1.5 && u.details.w<2.5 {base=mix(u.body.rgb,u.secondary.rgb*.6+vec3f(.30),smoothstep(-.05,.35,grain));}
        if u.details.w>2.5 {let chip=noise(p*39.);base=mix(base*.55,u.ink.rgb*.65+vec3f(.15),smoothstep(.65,.80,chip));}
        return mix(base,u.ink.rgb,pip_mask(p));
    }
    if kind==2 {
        base*=1.+material_grain(p)*.18;
        if u.details.w>2.5 {let patina=smoothstep(.43,.72,noise(p*7.)*.65+noise(p*31.)*.35);base=mix(base,vec3f(.025,.11,.095),patina*.65);}
        return mix(base,u.ink.rgb,coin_mark(p)*.25);
    }
    let uv=p.xy/vec2f(1.44,-1.44)+.5;
    let border=card_border(p.xy);
    if p.z<0.0 {
        base=mix(base,u.secondary.rgb,smoothstep(-1.0,1.0,p.y));
        let pattern=pow(max(0.0,cos(p.x*42.0)*cos(p.y*42.0)),16.0)*.035;
        base+=pattern;
        base*=1.+material_grain(p)*.20;
        return mix(base,u.ink.rgb,max(border,logo(vec2f(1.0-uv.x,uv.y))*.85));
    }
    base=mix(vec3f(.85,.82,.74),u.body.rgb,.035)*(1.+material_grain(p)*.14);
    let text=textureSampleLevel(label,linear_sampler,uv,0).r;
    return mix(base,mix(vec3f(.05,.04,.035),u.body.rgb,.35),max(text,border*.75));
}
fn shade(ro:vec3f,rd:vec3f,t:f32)->vec3f {
    let p=ro+rd*t;var nl=normal(p);
    if u.details.w>.5 && i32(u.viewport.z)!=0 {
        let e=.006;let grain=material_grain(p);
        let bump=vec3f(material_grain(p+vec3f(e,0,0))-grain,material_grain(p+vec3f(0,e,0))-grain,0);
        let strength=select(.08,.50,i32(u.viewport.z)==2 && u.details.w>1.5 && u.details.w<2.5);
        nl=normalize(nl-bump*strength*(1.-select(0.,coin_mark(p),i32(u.viewport.z)==2)));
    }
    let n=world_dir(nl);let v=-world_dir(rd);
    let rough=clamp(u.material.x+material_grain(p)*.12,.045,.9);let metal=select(0.0,1.0,i32(u.viewport.z)==2);
    let base=surface_color(p);let reflected=environment(reflect(-v,n),rough);
    let a=u.lighting.x;let light=normalize(vec3f(-.8*cos(a),1.6,1.8+.8*sin(a)));
    var col=brdf(n,v,light,base,metal,rough)*vec3f(3.1,2.8,2.5);
    col+=base*(1.0-metal)*environment(n,1.0)*.32;
    col+=reflected*fresnel(max(dot(n,v),0.0),mix(vec3f(.04),base,metal))*(1.0-rough*.65);
    if i32(u.viewport.z)==0 && u.material.y>.001 {
        let eta=1.0/u.material.w;let inside=refract(rd,nl,eta);var distance=.012;
        for(var i=0;i<48;i++){let d=shape(p+inside*distance);if d>=-.0005 && i>0{break;}distance+=max(-d*.75,.004);if distance>3.0{break;}}
        let exit=p+inside*distance;let en=normal(exit);var out_dir=refract(inside,-en,u.material.w);
        if length(out_dir)<.1 {out_dir=reflect(inside,-en);}
        let world_exit=world_dir(exit)*u.position.w+u.position.xyz;
        let transmitted=background(world_exit,world_dir(out_dir))*.65+environment(world_dir(out_dir),rough)*.35;
        var absorption=vec3f(0);var flecks=vec3f(0);
        let steps=select(4u,12u,u.lighting.w>1.5);
        for(var i=0u;i<steps;i++){
            let sample=p+inside*distance*(f32(i)+.5)/f32(steps);
            let density=clamp(cloud(sample)+material_grain(sample)*.8,0.,1.);let tint=mix(u.body.rgb,u.secondary.rgb,smoothstep(.32,.72,density));
            absorption+=(vec3f(1)-tint)*u.material.z*(.22+density*.85)*distance/f32(steps);
            let cells=sample*31.0;let speck=pow(max(0.0,1.0-length(fract(cells)-.5)*3.2),5.0)*step(.93,hash(floor(cells)));
            flecks+=u.ink.rgb*speck*u.details.z*36.0/f32(steps);
        }
        let trans=transmitted*exp(-absorption)+flecks;
        let f=pow((u.material.w-1.0)/(u.material.w+1.0),2.0);
        let boundary=fresnel(max(dot(n,v),0.0),vec3f(f));
        let textureBody=select(1.,select(.50,.28,u.details.w>2.5),u.details.w>.5);
        col=mix(col,trans*(1.0-boundary)+reflected*boundary,u.material.y*textureBody*(1.0-pip_mask(p)));
        if u.details.w>1.5 && u.details.w<2.5 {col+=environment(reflect(-v,n),.26)*u.secondary.rgb*pow(1.-abs(dot(n,v)),2.)*.35;}
    }
    return col*u.lighting.y;
}
@vertex fn vs(@builtin(vertex_index) i:u32)->@builtin(position) vec4f {
    let x=f32((i<<1u)&2u);let y=f32(i&2u);return vec4f(x*2.0-1.0,y*2.0-1.0,0,1);
}
fn render_pixel(pixel:vec2f)->vec4f {
    let xy=(pixel/u.viewport.xy*2.0-1.0)*vec2f(u.viewport.x/u.viewport.y,-1);
    var eye=select(vec3f(0,.75,4.7),vec3f(0,0,4.76),u.viewport.z==1.);let forward=normalize(vec3f(0,0,0)-eye);let right=normalize(cross(forward,vec3f(0,1,0)));let up=cross(right,forward);
    var ray=normalize(forward*u.lighting.z+right*xy.x+up*xy.y);
    if u.options.w>.5 && u.options.w<1.5 {eye=vec3f(xy.x*6.7,8.0,-xy.y*6.7);ray=vec3f(0,-1,0);}
    if u.options.w>1.5 {eye=vec3f(xy*1.35,5.);ray=vec3f(0,0,-1);}
    let ro=local(eye);let rd=qrot(vec4f(-u.rotation.xyz,u.rotation.w),ray);let t=trace(ro,rd);
    var color=background(eye,ray);
    var obstacle=-1.0;
    if u.options.w>.5 && u.options.w<1.5 {
        obstacle=obstacle_trace(eye,ray);
        if obstacle>=0.0 {let p=eye+ray*obstacle;let e=.003;let n=normalize(vec3f(obstacle_distance(p+vec3f(e,0,0))-obstacle_distance(p-vec3f(e,0,0)),obstacle_distance(p+vec3f(0,e,0))-obstacle_distance(p-vec3f(0,e,0)),obstacle_distance(p+vec3f(0,0,e))-obstacle_distance(p-vec3f(0,0,e))));color=obstacle_color(p)*(.65+.35*max(dot(n,normalize(vec3f(-.6,1,.4))),0.0));}
    }
    if t>=0.0 && (obstacle<0.0 || t*u.position.w<obstacle) {color=shade(ro,rd,t);}
    color=color/(color+vec3f(.7));
    if t>=0.0 && u.viewport.z<.5 && u.options.y>.5 {
        let mark=pip_mask(ro+rd*t);let luma=dot(color,vec3f(.2126,.7152,.0722));
        let ink=select(u.ink.rgb,select(vec3f(.015),vec3f(.95),luma<.45),abs(dot(u.ink.rgb,vec3f(.2126,.7152,.0722))-luma)<.35);
        color=mix(color,ink,mark*.96);
    }
    if t<0.0 && obstacle<0.0 && u.backdrop.w<-.5{return vec4f(0);}
    if t<0.0 && obstacle<0.0 && u.backdrop.w>.5{return vec4f(u.backdrop.rgb,1);}
    return vec4f(pow(max(color,vec3f(0)),vec3f(1.0/2.2)),1);
}
@fragment fn fs(@builtin(position) pixel:vec4f)->@location(0) vec4f {
    if u.lighting.w<2.0{return render_pixel(pixel.xy);}
    let color=render_pixel(pixel.xy+vec2f(-.25,-.25))+render_pixel(pixel.xy+vec2f(.25,-.25))+render_pixel(pixel.xy+vec2f(-.25,.25))+render_pixel(pixel.xy+vec2f(.25,.25));
    return color*.25;
}
