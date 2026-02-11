// Print Node Version - Example workflow
import { getAction } from "../../generated/index.ts";

const setupNode = getAction("actions/setup-node@v4");

const workflow = {
  name: "Print Node Version",
  on: {
    push: {
      branches: ["main"],
    },
  },
  jobs: {
    build: {
      "runs-on": "ubuntu-latest",
      steps: [
        setupNode({
          name: "Setup Node.js",
          id: "setup_node",
          with: {
            "node-version": "20",
          },
        }),
        {
          name: "Print Node.js version",
          run:
            'echo "Node.js version: ${{ steps.setup_node.outputs.node-version }}"',
        },
      ],
    },
  },
};

console.log(JSON.stringify(workflow, null, 2));
