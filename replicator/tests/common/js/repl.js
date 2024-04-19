const readline = require('readline');
const { AsyncQueue } = require('./utils.js');

module.exports.repl = async function() {
  const queue = new AsyncQueue();

  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    terminal: false
  });

  rl.on('line', function(chunk) {{
    console.log('got some stdout');
    queue.push(chunk);
  }});

  console.log('start loop');
  for await (line of queue) {
    eval(line.toString());
  }
  rl.close();
}
