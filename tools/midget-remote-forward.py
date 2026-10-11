#!/usr/bin/env python3
"""Expose Midget Remote on Titan's LAN, preserving HTTP/WebSocket authentication."""
import asyncio, os
async def client(reader,writer):
    upstream=None
    try:
        source,upstream=await asyncio.wait_for(asyncio.open_connection(os.getenv('MIDGET_REMOTE_HOST','192.168.20.140'),12000),5)
        async def copy(src,dst):
            while data:=await src.read(65536):
                dst.write(data);await dst.drain()
        tasks=[asyncio.create_task(copy(reader,upstream)),asyncio.create_task(copy(source,writer))]
        try: await asyncio.wait(tasks,return_when=asyncio.FIRST_COMPLETED)
        finally:
            for task in tasks: task.cancel()
            await asyncio.gather(*tasks,return_exceptions=True)
    except (OSError,asyncio.TimeoutError): pass
    finally:
        writer.close()
        if upstream: upstream.close()
async def main():
    server=await asyncio.start_server(client,os.getenv('MIDGET_REMOTE_BIND','192.168.1.12'),12000)
    async with server: await server.serve_forever()
asyncio.run(main())
