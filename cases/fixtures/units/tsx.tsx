import React from "react";

// JSX components; this comment is not a unit.
export function Hello(props: { name: string }) {
  return <div className="hi">{props.name}</div>;
}

export const Bye = () => <span>bye</span>;

const Title = function () {
  return <h1>Title</h1>;
};

var nums = function* () {
  yield 1;
}, label = "näme";

export class Panel extends React.Component {
  state = { open: false };
  render() {
    return <section>{this.state.open ? "open" : "closed"}</section>;
  }
}

interface Props {
  title?: string;
}

type Mode = "a" | "b";

Hello({ name: "x" });
