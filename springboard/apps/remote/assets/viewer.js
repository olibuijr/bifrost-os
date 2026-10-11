import RFB from './novnc/core/rfb.js';
const $=id=>document.getElementById(id);
let rfb,connected=false,explicitDisconnect=false,pinch=null,gesturePan=null,mousePan=null,composing=false,toastTimer;
const screen=$('screen');
function toast(message){$('toast').textContent=message;$('toast').hidden=false;clearTimeout(toastTimer);toastTimer=setTimeout(()=>{$('toast').hidden=true;},3500);}
function controls(){for(const id of ['fit','minus','actual','plus','mode','keyboard','clipboard','disconnect'])$(id).disabled=!connected;$('indicator').classList.toggle('connected',connected);}
function zoomLabel(){if(!rfb)return;$('actual').textContent=`${Math.round(rfb.viewportScale*100)}%`;$('fit').classList.toggle('active',rfb.scaleViewport);}
function zoom(scale,x,y){if(!connected)return;const r=screen.getBoundingClientRect();rfb.zoomViewport(scale,x??r.left+r.width/2,y??r.top+r.height/2);zoomLabel();}
function connect(){
 if(rfb)rfb.disconnect();screen.replaceChildren();explicitDisconnect=false;connected=false;controls();$('overlay').hidden=false;$('overlay-title').textContent='Connecting';$('overlay-message').textContent='Opening your BifrOSt desktop…';$('reconnect').hidden=true;$('login').hidden=true;$('status').textContent='Connecting…';
 rfb=new RFB(screen,`${location.protocol==='https:'?'wss':'ws'}://${location.host}/websockify`);const current=rfb;
 rfb.scaleViewport=true;rfb.resizeSession=false;rfb.showDotCursor=true;rfb.qualityLevel=6;rfb.compressionLevel=2;
 rfb.addEventListener('connect',()=>{if(current!==rfb)return;connected=true;$('overlay').hidden=true;$('status').textContent='Connected';controls();applyMode();requestAnimationFrame(zoomLabel);});
 rfb.addEventListener('disconnect',event=>{if(current!==rfb)return;connected=false;controls();$('status').textContent='Disconnected';$('overlay').hidden=false;$('overlay-title').textContent=explicitDisconnect?'Disconnected':'Connection lost';$('overlay-message').textContent=explicitDisconnect?'Your desktop stays running.':event.detail.clean?'The desktop session ended.':'Check the desktop connection, then reconnect.';$('reconnect').hidden=false;$('login').hidden=false;});
 rfb.addEventListener('clipboard',event=>{$('clipboard-text').value=event.detail.text;toast('Desktop clipboard received');});
 rfb.addEventListener('securityfailure',()=>{toast('Session expired. Sign in again.');$('login').hidden=false;});
}
function applyMode(){if(!rfb)return;rfb.viewOnly=$('mode').value!=='control';rfb.dragViewport=false;$('status').textContent=connected?({control:'Connected',pan:'Pan mode',view:'View only'}[$('mode').value]):'Disconnected';}
$('mode').addEventListener('change',applyMode);
$('fit').onclick=()=>{rfb.clipViewport=false;rfb.scaleViewport=true;zoomLabel();};
$('actual').onclick=()=>zoom(1);$('minus').onclick=()=>zoom(rfb.viewportScale/1.25);$('plus').onclick=()=>zoom(rfb.viewportScale*1.25);
$('reconnect').onclick=connect;$('disconnect').onclick=()=>{explicitDisconnect=true;rfb.disconnect();};
if(!document.fullscreenEnabled&&!document.webkitFullscreenEnabled)$('fullscreen').hidden=true;
$('fullscreen').onclick=async()=>{try{if(document.fullscreenElement||document.webkitFullscreenElement)await(document.exitFullscreen?.()??document.webkitExitFullscreen?.());else await($('viewer').requestFullscreen?.()??$('viewer').webkitRequestFullscreen?.());}catch{toast('Fullscreen is unavailable in this browser.');}};
for(const name of ['fullscreenchange','webkitfullscreenchange'])document.addEventListener(name,()=>{$('fullscreen').textContent=(document.fullscreenElement||document.webkitFullscreenElement)?'Exit fullscreen':'Fullscreen';requestAnimationFrame(zoomLabel);});
new ResizeObserver(()=>requestAnimationFrame(()=>requestAnimationFrame(zoomLabel))).observe(screen);
// Intercept noVNC's recognized gestures before its remote-input handler. Local
// scale lives in Display, so cursor coordinates remain correct after zooming.
for(const type of ['gesturestart','gesturemove','gestureend'])screen.addEventListener(type,event=>{
 const d=event.detail,kind=d.type,panMode=$('mode').value!=='control';
 if(kind==='pinch'){
  event.stopImmediatePropagation();event.preventDefault();const magnitude=Math.hypot(d.magnitudeX,d.magnitudeY);
  if(type==='gesturestart')pinch={magnitude:Math.max(1,magnitude),scale:rfb.viewportScale};
  else if(type==='gesturemove'&&pinch)zoom(pinch.scale*magnitude/pinch.magnitude,d.clientX,d.clientY);
  else if(type==='gestureend')pinch=null;
 }else if(kind==='twodrag'||(panMode&&kind==='drag')){
  event.stopImmediatePropagation();event.preventDefault();
  if(type==='gesturestart')gesturePan={x:d.clientX,y:d.clientY};
  else if(type==='gesturemove'&&gesturePan){rfb.panViewport(gesturePan.x-d.clientX,gesturePan.y-d.clientY);gesturePan={x:d.clientX,y:d.clientY};}
  else if(type==='gestureend')gesturePan=null;
 }else if(panMode){event.stopImmediatePropagation();event.preventDefault();}
},true);
screen.addEventListener('mousedown',event=>{if($('mode').value==='control')return;event.stopImmediatePropagation();event.preventDefault();mousePan={x:event.clientX,y:event.clientY};},true);
screen.addEventListener('mousemove',event=>{if(!mousePan)return;event.stopImmediatePropagation();rfb.panViewport(mousePan.x-event.clientX,mousePan.y-event.clientY);mousePan={x:event.clientX,y:event.clientY};},true);
window.addEventListener('mouseup',()=>{mousePan=null;},true);
screen.addEventListener('wheel',event=>{if(event.ctrlKey){event.stopImmediatePropagation();event.preventDefault();zoom(rfb.viewportScale*Math.exp(-event.deltaY*.002),event.clientX,event.clientY);}else if($('mode').value!=='control'){event.stopImmediatePropagation();event.preventDefault();rfb.panViewport(event.deltaX,event.deltaY);}}, {capture:true,passive:false});
$('clipboard').onclick=()=>{$('clipboard-dialog').showModal();$('clipboard-text').focus();};$('clipboard-close').onclick=()=>$('clipboard-dialog').close();$('clipboard-send').onclick=()=>{rfb.clipboardPasteFrom($('clipboard-text').value);toast('Clipboard sent to desktop');};
function key(keysym){if(connected&&$('mode').value==='control')rfb.sendKey(keysym,'');}
$('keyboard').onclick=()=>{if($('mode').value!=='control'){toast('Choose Control mode to type.');return;}$('keyboard-panel').hidden=!$('keyboard-panel').hidden;if(!$('keyboard-panel').hidden)$('keyboard-input').focus();};$('keyboard-close').onclick=()=>{$('keyboard-input').blur();$('keyboard-panel').hidden=true;};$('send-tab').onclick=()=>key(0xff09);$('send-escape').onclick=()=>key(0xff1b);
const input=$('keyboard-input');function sendText(){if(composing)return;for(const char of input.value){const point=char.codePointAt(0);key(point<=0xff?point:0x01000000|point);}input.value='';}input.addEventListener('compositionstart',()=>{composing=true;});input.addEventListener('compositionend',()=>{composing=false;sendText();});input.addEventListener('input',sendText);input.addEventListener('keydown',event=>{const codes={Enter:0xff0d,Backspace:0xff08,Tab:0xff09,Escape:0xff1b,ArrowLeft:0xff51,ArrowUp:0xff52,ArrowRight:0xff53,ArrowDown:0xff54};if(codes[event.key]){event.preventDefault();key(codes[event.key]);}});
connect();
