"""BifrOSt browser desktop bridge. VNC stays on a private Unix socket."""
import asyncio, json, os, pathlib, secrets, signal, subprocess, sys
from aiohttp import web, WSMsgType
ROOT = pathlib.Path(__file__).resolve().parent
DATA = pathlib.Path(sys.argv[1]); DATA.mkdir(parents=True, exist_ok=True)
os.umask(0o077)
secret_file = DATA / 'password'
if not secret_file.exists(): secret_file.write_text(secrets.token_urlsafe(12))
SECRET = secret_file.read_text().strip()
SOCKET = str(DATA / 'vnc.sock')
STATUS = DATA / 'status.json'
sessions = set()

def state(status, error=''):
    temp = DATA / 'status.tmp'
    temp.write_text(json.dumps(dict(status=status,error=error,password=SECRET)))
    temp.replace(STATUS)

async def login(request):
    form = await request.post()
    if not secrets.compare_digest(str(form.get('password','')), SECRET):
        return web.Response(text='Incorrect password',status=403)
    token=secrets.token_urlsafe(32); sessions.add(token)
    response=web.HTTPFound('/desktop.html')
    response.set_cookie('remote',token,httponly=True,samesite='Strict')
    return response

async def websocket(request):
    if request.cookies.get('remote') not in sessions: raise web.HTTPForbidden()
    if request.headers.get('Origin') != f'{request.scheme}://{request.host}': raise web.HTTPForbidden()
    try: reader,writer=await asyncio.open_unix_connection(SOCKET)
    except OSError: raise web.HTTPServiceUnavailable(text='Desktop is restarting; reconnect shortly')
    ws=web.WebSocketResponse(protocols=['binary'],max_msg_size=16*1024*1024)
    await ws.prepare(request)
    async def incoming():
        async for msg in ws:
            if msg.type == WSMsgType.BINARY:
                writer.write(msg.data); await writer.drain()
            elif msg.type in (WSMsgType.ERROR, WSMsgType.CLOSE): break
    async def outgoing():
        while data:=await reader.read(65536): await ws.send_bytes(data)
    jobs=[asyncio.create_task(incoming()),asyncio.create_task(outgoing())]
    try: await asyncio.wait(jobs,return_when=asyncio.FIRST_COMPLETED)
    finally:
        for job in jobs: job.cancel()
        await asyncio.gather(*jobs,return_exceptions=True)
        writer.close(); await writer.wait_closed(); await ws.close()
    return ws

async def main():
    app=web.Application(client_max_size=4096)
    app.router.add_post('/login',login); app.router.add_get('/websockify',websocket)
    app.router.add_get('/',lambda r:web.FileResponse(ROOT/'index.html'))
    app.router.add_get('/desktop.html',lambda r:web.FileResponse(ROOT/'desktop.html'))
    for asset in ('viewer.js','viewer.css'):
        app.router.add_get('/'+asset,lambda r,name=asset:web.FileResponse(ROOT/name))
    app.router.add_static('/novnc/',ROOT/'novnc')
    runner=web.AppRunner(app,access_log=None); await runner.setup()
    await web.TCPSite(runner,'0.0.0.0',12000).start()
    stop=asyncio.Event(); loop=asyncio.get_running_loop()
    for sig in (signal.SIGTERM,signal.SIGINT): loop.add_signal_handler(sig,stop.set)
    config=DATA/'wayvnc.conf'; config.write_text('enable_auth=false\n')
    proc=None
    try:
        while not stop.is_set():
            pathlib.Path(SOCKET).unlink(missing_ok=True)
            try:
                state('Waiting for desktop')
                session=json.loads(pathlib.Path('/run/bifrost/user/session.json').read_text())
                env=os.environ.copy(); env.update(session)
                proc=await asyncio.create_subprocess_exec('wayvnc','-S',str(DATA/'wayvncctl'),'-C',str(config),'-u','-R','-f','30','-n','BifrOSt',SOCKET,env=env)
                for _ in range(100):
                    if stop.is_set(): break
                    if proc.returncode is not None: raise RuntimeError('Waiting for Wayland session')
                    if pathlib.Path(SOCKET).exists(): break
                    await asyncio.sleep(.1)
                else: raise RuntimeError('Waiting for WayVNC socket')
                if not stop.is_set():
                    state('Serving')
                    stopped=asyncio.create_task(stop.wait()); exited=asyncio.create_task(proc.wait())
                    await asyncio.wait([stopped,exited],return_when=asyncio.FIRST_COMPLETED)
                    stopped.cancel(); exited.cancel()
                    await asyncio.gather(stopped,exited,return_exceptions=True)
            except (OSError,ValueError,RuntimeError) as error:
                state('Waiting for desktop',str(error))
            finally:
                if proc is not None and proc.returncode is None:
                    proc.terminate(); await proc.wait()
                proc=None
                pathlib.Path(SOCKET).unlink(missing_ok=True)
            if not stop.is_set():
                try: await asyncio.wait_for(stop.wait(),3)
                except asyncio.TimeoutError: pass
    finally:
        await runner.cleanup()
        state('Stopped')

if __name__=='__main__':
    try: asyncio.run(main())
    except Exception as e: state('Error',str(e)); raise
