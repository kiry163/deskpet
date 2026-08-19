import ReactDOM from "react-dom/client";
import App from "./App";

// 注：不用 StrictMode——dev 下双挂载会导致 PIXI 重复初始化（原型阶段）
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <App />,
);
