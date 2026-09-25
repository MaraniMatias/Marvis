export function exactPayloadCommand(begin, end, byteCount) {
  if (!Number.isSafeInteger(byteCount) || byteCount < 1) {
    throw new RangeError("Payload byte count must be a positive safe integer");
  }
  const toHex = (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return (
    `node -e "const fs=require('node:fs'),b=Buffer.alloc(65536,120),w=buf=>{let o=0;while(o<buf.length){const n=fs.writeSync(1,buf,o,buf.length-o);if(!n)throw Error('short PTY write');o+=n}};` +
    `w(Buffer.from('${toHex(begin)}','hex'));` +
    `for(let n=${byteCount};n>0;n-=b.length)w(b.subarray(0,Math.min(n,b.length)));` +
    `w(Buffer.from('${toHex(end)}','hex'));"`
  );
}
