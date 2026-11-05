const port = process.argv[2];
const deadline = Date.now() + 25_000;

async function target() {
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    return list.find((item) => item.type === 'page' && item.url.includes('browser-upload'));
  } catch {
    return null;
  }
}

let page = null;
while (Date.now() < deadline) {
  page = await target();
  if (page) break;
  await new Promise((resolve) => setTimeout(resolve, 200));
}
if (!page) {
  console.log('NO_PAGE');
  process.exit(2);
}

const socket = new WebSocket(page.webSocketDebuggerUrl);
let nextId = 1;
const pending = new Map();
socket.addEventListener('message', (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message.result);
    pending.delete(message.id);
  }
});
await new Promise((resolve, reject) => {
  socket.addEventListener('open', resolve, { once: true });
  socket.addEventListener('error', reject, { once: true });
});
const evaluate = (expression) =>
  new Promise((resolve) => {
    const id = nextId++;
    pending.set(id, (result) => resolve(result?.result?.value));
    socket.send(
      JSON.stringify({
        id,
        method: 'Runtime.evaluate',
        params: { expression, awaitPromise: true, returnByValue: true },
      }),
    );
  });

while (Date.now() < deadline) {
  const text = await evaluate(`document.body?.innerText ?? ''`);
  if (text.includes('UPLOADED') || text.includes('ERROR')) {
    console.log(text);
    socket.close();
    process.exit(text.includes('UPLOADED') ? 0 : 1);
  }
  await new Promise((resolve) => setTimeout(resolve, 250));
}
console.log('TIMEOUT');
process.exit(3);
