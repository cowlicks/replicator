const Hypercore = require('hypercore');
const RAM = require('random-access-memory');

const logYellow = f => {
  process.stdout.write('\x1b[33m');
  const out =f();
  process.stdout.write('\x1b[0m');
  return out;
}

const logBlue = f => {
  process.stdout.write('\x1b[34m');
  const out =f();
  process.stdout.write('\x1b[0m');
  return out;
}
const l = (...x) => [console.log(...x), x[0]][1];

const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const pause = () => sleep(500);

const noInitialData = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();
  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  await pause();

  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);

  await pause();
  /* Writer and Reader both send syncs */
  l('\n\nsent messages\n\n')
  l(a.replicator.peers[0].tx_msgs);
  l(b.replicator.peers[0].tx_msgs);
});

const writerWithInitialData = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();
  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  await pause();
  await a.append('0');
  await pause();
  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await pause();
  /* Writer and Reader both send syncs */
  l('\n\nsent messages\n\n')
  l(a.replicator.peers[0].tx_msgs);
  l(b.replicator.peers[0].tx_msgs);
});

const writerNoInitialDataAddsData = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();

  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();

  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await pause();
  await a.append('0');
  await pause();
  /* Writer and Reader both send syncs */
  l('\n\nsent messages\n\n')
  l(a.replicator.peers[0].tx_msgs);
  l(b.replicator.peers[0].tx_msgs);
  await pause();
});

(async () => {
  //await noInitialData();
  await writerWithInitialData();
  //await writerNoInitialDataAddsData();
})()
