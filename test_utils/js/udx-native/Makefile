
node_modules: package.json
	npm install

# this is the "generate" step
build: node_modules
	npx bare-make generate --platform linux --arch x64 --debug

# This is the "generate" step. There is probably better way to check than just
# "build/udx-native.node" but this works and it is simple
build/udx-native.node: build
	npx bare-make build

# This is the "install" step. There is probably better way to check than just
# "prebuilds", but this works and it's simple
prebuilds: build/udx-native.node
	npx bare-make install

clean:
	rm -rf build prebuilds node_modules package-lock.json


.PHONY: clean
