const Hypercore = require('../../../../../js/core');
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
const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const pause = () => sleep(500);


const initial_sync = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();
  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);

  await pause();
});

const one_block_before = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();
  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  await pause();
  await a.append('0');
  await pause();
  console.log("do connect");;
  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await pause();
});

const arr_equal = (a, b) => {
  
  const al = a.length;
  const bl = b.length;

  if (al != bl) {
    return false;
  }
  for (let i = 0; i < al; i++) {
    if (a[i] != b[i]) {
      return false;
    }
  }
  return true
}

const append_many_foreach_reader_update_reader_get = (async () => {
  const data = [[0], [1], [2]];

  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();

  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();

  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await pause();
  for (let i = 0; i < data.length; i += 1) {
    
    await a.append(Buffer.from(data[i]));
    while (true) {
      let l = (await b.info()).length;
      if (l == i + 1) {
        break
      }
      await pause();
    }
    while (true) {
      let res = await b.get(i);
      if (arr_equal(res, Buffer.from(data[i]))) {
        break
      }
      await pause();
    }
  }
});

const zeroBlocks = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();

  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();

  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await pause();
  await b.update({wait: true});
  await pause();
  return [a, b];
});


const zeroBlocksNoUp = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();

  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();

  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await pause();
  return [a, b];
});



const oneBlocksNoUp = (async () => {
 const [a, b] = await zeroBlocks();

  await a.append('0');
  await pause();

  while (true) {
    if (b.length == 1) {
      break
    }
    await pause();
  }
  return [a, b]
});
const oneBlocks = (async () => {
 const [a, b] = await zeroBlocks();

  await a.append('0');
  await pause();

  while (true) {
    await b.update({wait: true});
    if (b.length == 1) {
      break
    }
    await pause();
  }
  return [a, b]
});

const twoBlocks = (async () => {
  const [a, b] = await oneBlocks();

  await a.append('1');
  while (true) {
    await b.update({wait: true});
    if (b.length == 2) {
      break
    }
    await pause();
  }
  await pause();
});


(async () => {
  //await noInitialData();
  //await one_block_before();
  //await initial_sync();
  await append_many_foreach_reader_update_reader_get();
})()
