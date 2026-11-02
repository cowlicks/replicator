const createTestnet = require('../testnet')
const b4a = require('b4a')
const DHT = require('../')

const l = (...x) => [console.log(...x), x[0]][1]

const bootstrap = [];
const n_nodes = 64;

(async () => {
  //  n_nodes connected nodes, all connected through the first node made as the bootstrap node
  let testNet = await createTestnet(n_nodes, { bootstrap });


  let ann_node = testNet.nodes[testNet.nodes.length - 1];
  const topic = b4a.alloc(32);
  topic.write('hello', 0);

  // do an announce for ann_node of our topic
  let query = await ann_node.announce(topic, ann_node.defaultKeyPair);

  await query.finished();

  let peer = testNet.nodes[0];
  let lq = await peer.lookup(topic);
  let res = (async () => {
    for await ( let msg of lq ) {
      let msg_pub_key = msg.peers.pop()?.publicKey;
      if (Buffer.compare(msg_pub_key, ann_node.defaultKeyPair.publicKey) == 0) {
        return msg_pub_key;
      }
    }
  })();
  await res;
  let target = await res;
  if (!target) {
    throw 666
  }
  l(JSON.stringify([...target]))

  //l('ann nod pk', ann_node.defaultKeyPair.publicKey);

  //await testNet.destroy()
})()
