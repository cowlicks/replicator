const Hypercore = require('../../../../../js/core');
const RAM = require('random-access-memory');

const path = require('path');
const readline = require('readline');

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
const slow = (ms = 500) => sleep(ms);

const ll = (x) => [console.log(x), x][1]

const curr_line = () => {
  const originalPrepareStackTrace = Error.prepareStackTrace;
  Error.prepareStackTrace = (_, stack) => stack;
  const callee = new Error().stack[4];
  Error.prepareStackTrace = originalPrepareStackTrace;
  const relativeFileName = path.relative(process.cwd(), callee.getFileName());
  return `[ ${callee.getLineNumber()} ] -- ${relativeFileName}`
}

let p_counter = 0
const pause = () => new Promise((resolve, ..._x) => {
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout
  });

  let loc = curr_line();
  let msg = `${loc} | Press Enter to continue...`;

  setTimeout(() => console.log(msg), 500);
  rl.question(msg, () => {
    resolve();
    p_counter += 1;
    rl.close();
  })
})

const make_connected_slave = async (master, master_is_initiator, ...x) => {
  const slave = new Hypercore(RAM, master.key, ...x);
  await slave.ready();

  const m_to_s = master.replicate(master_is_initiator, { keepAlive: false });
  const s_to_m = slave.replicate(!master_is_initiator, { keepAlive: false });
  m_to_s.pipe(s_to_m).pipe(m_to_s);
  await slave.ready();
  return slave
}

const assert_core_get = async (core, index, expected) => {
  const res = await core.get(index);
  if (!arr_equal(Buffer.from(expected), res)) {
    throw new Error("sntshshsh");
  }
  return res
}



const initial_sync = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();
  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);

  await slow();
});

const one_block_before = (async () => {
  const a = new Hypercore(RAM, undefined, {name: 'Writer'});
  await a.ready();
  const b = new Hypercore(RAM, a.key, {name: 'Reader'});
  await b.ready();
  await slow();
  await a.append('0');
  await slow();
  console.log("do connect");;
  const s1 = a.replicate(false, { keepAlive: false });
  const s2 = b.replicate(true, { keepAlive: false });
  s1.pipe(s2).pipe(s1);
  await slow();
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
  await slow();
  for (let i = 0; i < data.length; i += 1) {
    
    await a.append(Buffer.from(data[i]));
    while (true) {
      let l = (await b.info()).length;
      if (l == i + 1) {
        break
      }
      await slow();
    }
    while (true) {
      let res = await b.get(i);
      if (arr_equal(res, Buffer.from(data[i]))) {
        break
      }
      await slow();
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
  await slow();
  await b.update({wait: true});
  await slow();
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
  await slow();
  return [a, b];
});



const oneBlocksNoUp = (async () => {
 const [a, b] = await zeroBlocks();

  await a.append('0');
  await slow();

  while (true) {
    if (b.length == 1) {
      break
    }
    await slow();
  }
  return [a, b]
});
const oneBlocks = (async () => {
 const [a, b] = await zeroBlocks();

  await a.append('0');
  await slow();

  while (true) {
    await b.update({wait: true});
    if (b.length == 1) {
      break
    }
    await slow();
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
    await slow();
  }
  await slow();
});

/*
(async () => {
  //await noInitialData();
  //await one_block_before();
  //await initial_sync();
  //await append_many_foreach_reader_update_reader_get();
  await small_path_topology()
})()
*/

const what_does_get_wo_len_for_block = (async () => {
  const master = new Hypercore(RAM, undefined, {name: 'MASTER'});
  await master.ready();

  await master.append(Buffer.from([0]));
  await pause();

  const first_peer = await make_connected_slave(master, false, {name: "FIRST"});
  await pause();

  assert_core_get(first_peer, 0, [0]);
  await pause();
  console.log(await first_peer.get(1));
  await pause();
});

const small_path_topology = (async () => {
  const master = new Hypercore(RAM, undefined, {name: 'MASTER'});
  await master.ready();

  await master.append(Buffer.from([0]));
  await pause();

  const first_peer = await make_connected_slave(master, false, {name: "FIRST"});
  await pause();

  assert_core_get(first_peer, 0, [0]);
  await pause();

  const second_peer = await make_connected_slave(first_peer, false, {name: "SECOND"});
  await pause();

  assert_core_get(second_peer, 0, [0]);
  await pause();

  await master.append(Buffer.from([1]));
  await pause();

  assert_core_get(first_peer, 1, [1]);
  await pause();

  assert_core_get(second_peer, 1, [1]);
})();
