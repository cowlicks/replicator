const readline = require('readline');
const { AsyncQueue } = require('./utils.js');

module.exports.repl = async function() {
  const queue = new AsyncQueue();
  process.stdin.on('data', (chunk) => {
    queue.push(chunk)
  });

  for await (line of queue) {
    eval(line.toString());
  }

  process.stdin.pause();

}
