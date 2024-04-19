const log = (x) => [console.log(x), x][1]

function Deferred() {
  const o = {};
  const p = new Promise((resolve, reject) => Object.assign(o, { resolve, reject }));
  const rejectAndCatch = (rejectionReason, catchFunc = () => {}) => {
    (o).reject(rejectionReason);
    p.catch(catchFunc);
  };
  return Object.assign(p, o, { rejectAndCatch });
}

const QueueDone = Symbol('QueueDone');

function _box(x) {
  return [x];
}

function _unbox(x) {
  return x[0];
}

class AsyncQueue {

  constructor() {
    this._queue = [];
    this._waiter = null;
    this._done = false;
  }

  async get() {
    return _unbox(await this._get());
  }

  _get() {
    if (this._queue.length) {
      return this._queue.shift();
    }
    if (!this._waiter) {
      this._waiter = Deferred();
    }
    return this._waiter;
  }

  get size() {
    return this._queue.length;
  }

  _addFunc(x, func) {
    if (this._done) {
      throw new Error('Cannot push on a done queue');
    }
    if (this._waiter) {
      this._waiter.resolve(_box(x));
      this._waiter = null;
    } else {
      func(_box(x));
    }
  }

  push(...stuff) {
    stuff.forEach((x) => {
      this._addFunc(x, (y) => this._queue.push(y));
    });
    return this;
  }

  unshift(...stuff) {
    stuff.forEach((x) => {
      this._addFunc(x, (y) => this._queue.unshift(y));
    });
    return this;
  }

  done() {
    this._done = true;
    if (!this._waiter) {
      this._waiter = Deferred();
    }
    this._waiter.resolve(QueueDone);
  }

  [Symbol.asyncIterator]() {
    const self = this;
    return {
      async next() {
        const value = await self._get();
        if (value === QueueDone) {
          return { done: true };
        }

        return { done: false, value: _unbox(value) };
      },
    };
  }
}

module.exports.AsyncQueue = AsyncQueue;
Object.assign(module.exports, {
  AsyncQueue,
  Deferred,
});
