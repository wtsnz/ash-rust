// A WebSocket client that sends headers, which both desks read a subscriber's tenant and
// actor from, and Node's built-in WebSocket can't send. Text frames only: enough for
// `graphql-transport-ws`.

import { request } from "node:http";
import { randomBytes } from "node:crypto";
import type { Socket } from "node:net";

export type Ws = {
  send(text: string): void;
  close(): void;
};

export const connect = (
  url: string,
  headers: Record<string, string>,
  protocol: string,
  onText: (text: string) => void,
): Promise<Ws> =>
  new Promise((resolve, reject) => {
    const req = request(url.replace(/^ws/, "http"), {
      headers: {
        ...headers,
        Connection: "Upgrade",
        Upgrade: "websocket",
        "Sec-WebSocket-Version": "13",
        "Sec-WebSocket-Key": randomBytes(16).toString("base64"),
        "Sec-WebSocket-Protocol": protocol,
      },
    });
    req.on("error", reject);
    req.on("response", (res) => reject(new Error(`${url} answered ${res.statusCode}, not an upgrade`)));
    req.on("upgrade", (_res, socket: Socket, head: Buffer) => {
      socket.setNoDelay(true);
      let buffer = head;
      let message: Buffer[] = [];
      const write = (opcode: number, payload: Buffer) => {
        // Client frames are masked, as the protocol requires.
        const mask = randomBytes(4);
        const len = payload.length;
        const header = len < 126 ? Buffer.alloc(2) : len < 65536 ? Buffer.alloc(4) : Buffer.alloc(10);
        header[0] = 0x80 | opcode;
        if (len < 126) header[1] = 0x80 | len;
        else if (len < 65536) {
          header[1] = 0x80 | 126;
          header.writeUInt16BE(len, 2);
        } else {
          header[1] = 0x80 | 127;
          header.writeBigUInt64BE(BigInt(len), 2);
        }
        const masked = Buffer.alloc(len);
        for (let i = 0; i < len; i++) masked[i] = payload[i] ^ mask[i & 3];
        socket.write(Buffer.concat([header, mask, masked]));
      };
      socket.on("data", (chunk: Buffer) => {
        buffer = buffer.length ? Buffer.concat([buffer, chunk]) : chunk;
        while (buffer.length >= 2) {
          const fin = (buffer[0] & 0x80) !== 0;
          const opcode = buffer[0] & 0x0f;
          let len = buffer[1] & 0x7f;
          let offset = 2;
          if (len === 126) {
            if (buffer.length < 4) return;
            len = buffer.readUInt16BE(2);
            offset = 4;
          } else if (len === 127) {
            if (buffer.length < 10) return;
            len = Number(buffer.readBigUInt64BE(2));
            offset = 10;
          }
          if (buffer.length < offset + len) return;
          const payload = buffer.subarray(offset, offset + len);
          buffer = buffer.subarray(offset + len);
          if (opcode === 0x9) write(0xa, payload); // ping → pong
          else if (opcode === 0x8) socket.end();
          else if (opcode === 0x1 || opcode === 0x0) {
            message.push(Buffer.from(payload));
            if (fin) {
              onText(Buffer.concat(message).toString("utf8"));
              message = [];
            }
          }
        }
      });
      socket.on("error", () => {});
      resolve({
        send: (text) => write(0x1, Buffer.from(text, "utf8")),
        close: () => {
          write(0x8, Buffer.alloc(0));
          socket.end();
        },
      });
    });
    req.end();
  });
