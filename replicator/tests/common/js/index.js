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
const l = (...x) => [logBlue(() => console.log(...x)), x[0]][1];

const sleep = (ms) => new Promise(r => setTimeout(r, ms));

const cmpWithRs = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer', log: (...x)  => logYellow(() => console.log(...x))});
  await a.ready();
  l('--A ready');
  await a.append(['a', 'b', 'c']);

  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  l('--B ready');

  const s1 = a.replicate(true, { keepAlive: false });
  l('--A replication stream created')
  const s2 = b.replicate(false, { keepAlive: false });
  l('--B replication stream created')
  s1.pipe(s2).pipe(s1);
  l('--A and B piped together');

  await sleep(1);
  b.update({wait: true});
  await sleep(1);

  if ((await b.info()).length != (await a.info()).length) {
    throw 'fail'
  }
  l('--------------B udate complete. Adding more data to A------------------');

  await a.append(['d']);
  await sleep(1);
  b.update({wait: true});

  if ((await b.info()).length != (await a.info()).length) {
    throw 'fail'
  }
});

//const doesWriterBroadcast = (async () => {
const doesWriterBroadcast = (async () => {
  l('--- JUST SETUP ---');
  const a = new Hypercore(RAM, undefined, {name: 'Writer', log: (...x)  => logYellow(() => console.log(...x))});
  await a.ready();
  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await sleep(1e3);
  /* Writer and Reader both send syncs */
  l(b.replicator.peers[0].tx_msgs);
  //l(b.replicator.rx_msgs);
  l(a.replicator.peers[0].tx_msgs);
  //l(a.replicator.rx_msgs);
  l('----- READY --------');
  const res = await a.append('0');
  await sleep(1e3);
  l('----- APPENDED --------');
  l(await b.get(0));
  //await sleep(1e3);
  l('----- GOTTEN --------');
  /* Writer and Reader both send syncs */
  //l(a.replicator.peers[0].tx_msgs);
  l(a.replicator.pmsgs());
  //l(b.replicator.rx_msgs);
  //l(b.replicator.peers[0].tx_msgs);
  //l(a.replicator.rx_msgs);
  await sleep(1e3);
})();


