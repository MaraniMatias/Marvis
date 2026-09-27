import { createApp } from "vue";
import App from "./App.vue";
import { installContextMenu } from "./lib/context-menu";
import "./marvis.css";
import "./style.css";

installContextMenu();
createApp(App).mount("#app");
