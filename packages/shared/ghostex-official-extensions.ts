/*
 * CDXC:Extensions 2026-08-30:
 * Settings has one Extensions page for everything a user can turn on or off:
 * the features Ghostex ships itself ("Official") and the audited third-party
 * extensions gxserver installs. The built-in features have no manifest and no
 * registry entry — they are plain settings keys — so this descriptor list is
 * what lets the Extensions page render them beside real extensions without
 * inventing an `origin`/`builtin` field on the gxserver wire contract.
 *
 * Every settings key here is inverted: the settings schema stores "hidden",
 * while the Extensions page shows "enabled". An entry is enabled when its key
 * is `false`; missing values use the settings defaults.
 */
import type { ghostexSettings } from './ghostex-settings';
import {
  PROJECT_WEBSITE_PROVIDERS,
  type ProjectWebsiteId,
  type ProjectWebsiteHiddenKey,
} from './ghostex-settings/project-websites';

type BooleanGhostexSettingsKey = {
  [Key in keyof ghostexSettings]-?: boolean extends ghostexSettings[Key] ? Key : never;
}[keyof ghostexSettings];

/**
 * The boolean visibility keys the Official entries own. `Extract` against the
 * settings type means a renamed or non-boolean key stops compiling here instead
 * of silently rendering a switch that writes nothing.
 */
export type GhostexOfficialExtensionSettingsKey = Extract<
  BooleanGhostexSettingsKey,
  | 'automateViewTabHidden'
  | 'botAutomationsHidden'
  | 'botsHidden'
  | 'browserViewTabHidden'
  | 'codeViewTabHidden'
  | 'devServersTitlebarButtonHidden'
  | 'docsViewTabHidden'
  | 'extensionsTitlebarButtonHidden'
  | 'gitActionsTitlebarButtonHidden'
  | 'helpTitlebarButtonHidden'
  | 'kanbanViewTabHidden'
  | 'notificationsTitlebarButtonHidden'
  | 'openDotsHidden'
  | 'openInTitlebarButtonHidden'
  | 'quickActionsTitlebarButtonHidden'
  | 'resourcesTitlebarButtonHidden'
  | 'storybookViewTabHidden'
  | ProjectWebsiteHiddenKey
  | 'terminalViewTabHidden'
  | 'tipsAndTricksTitlebarButtonHidden'
>;

/** Where an official entry appears in the app once it is enabled. */
export type GhostexOfficialExtensionPlacement = 'view' | 'titlebar-button' | 'sidebar';

/**
 * CDXC:Extensions 2026-09-24 DECISION:
 * User: the built-in extensions are grouped by category on the Settings Extensions page, replacing the two
 * "Views" / "Buttons and menus" buckets. The user rejected the "⋯ menu panels" label as ugly; that group is
 * "Menus and panels". The CEF runtime is not a descriptor, so the page adds its "Shared runtime" group itself.
 */
export type GhostexOfficialExtensionCategory =
  'project-websites' | 'code-and-files' | 'planning' | 'header-buttons' | 'menus-and-panels';

export const GHOSTEX_OFFICIAL_EXTENSION_CATEGORIES: readonly {
  id: GhostexOfficialExtensionCategory;
  label: string;
  /** The type shown on each card's footer and offered by the page's type filter. */
  typeLabel: string;
}[] = [
  { id: 'project-websites', label: 'Project websites', typeLabel: 'View' },
  { id: 'code-and-files', label: 'Code and files', typeLabel: 'View' },
  { id: 'planning', label: 'Planning and automation', typeLabel: 'View' },
  { id: 'header-buttons', label: 'Header buttons', typeLabel: 'Header button' },
  { id: 'menus-and-panels', label: 'Menus and panels', typeLabel: 'Menu item' },
];

export type GhostexOfficialExtensionId =
  | ProjectWebsiteId
  | 'automate'
  | 'botAutomations'
  | 'bots'
  | 'browser'
  | 'code'
  | 'devServers'
  | 'docs'
  | 'extensionsButton'
  | 'gitActions'
  | 'help'
  | 'kanban'
  | 'notifications'
  | 'openDots'
  | 'openIn'
  | 'quickActions'
  | 'resources'
  | 'storybook'
  | 'terminal'
  | 'tips';

export type GhostexOfficialExtension = {
  /**
   * CDXC:Titlebar 2026-09-21 DECISION:
   * User: Ask Ghostex, Tips & Tricks and Resources open as overlays from the ⋯ menu again (no longer view tabs), and stay app-wide: available in every project regardless of scope. An app-wide entry has an on/off switch and nothing else: no "where it appears" editor, because there is no project it could be narrowed to.
   */
  appWide?: true;
  category: GhostexOfficialExtensionCategory;
  description: string;
  id: GhostexOfficialExtensionId;
  placement: GhostexOfficialExtensionPlacement;
  /** The agent CLI (an `agent-cli-catalog.json` id) that must be installed for the entry to be offered at all. */
  requiresAgentCli?: string;
  /** The entry that must be enabled first: until it is, this entry's switch is shown but cannot be used. */
  requiresExtension?: GhostexOfficialExtensionId;
  settingsKey: GhostexOfficialExtensionSettingsKey;
  title: string;
};

export const GHOSTEX_OFFICIAL_EXTENSIONS: readonly GhostexOfficialExtension[] = [
  ...PROJECT_WEBSITE_PROVIDERS.map((provider): GhostexOfficialExtension => ({
    id: provider.id,
    title: provider.title,
    description: provider.description,
    settingsKey: provider.hiddenSettingsKey,
    placement: 'view',
    category: 'project-websites',
  })),
  {
    description:
      'Build, browse, and annotate your project’s components without a persistent development server. Appears only in projects with Storybook.',
    category: 'code-and-files',
    id: 'storybook',
    placement: 'view',
    settingsKey: 'storybookViewTabHidden',
    title: 'Storybook',
  },
  {
    description:
      'Explore, edit, and search your project in a familiar, full-featured workspace without ever leaving Ghostex.',
    category: 'code-and-files',
    id: 'code',
    placement: 'view',
    settingsKey: 'codeViewTabHidden',
    title: 'Code editor',
  },
  {
    description: 'Open websites alongside your project and keep useful pages organized without leaving Ghostex.',
    category: 'code-and-files',
    id: 'browser',
    placement: 'view',
    settingsKey: 'browserViewTabHidden',
    title: 'Browser',
  },
  {
    description: 'Plan upcoming work and track task progress at a glance.',
    category: 'planning',
    id: 'kanban',
    placement: 'view',
    settingsKey: 'kanbanViewTabHidden',
    title: 'Kanban',
  },
  {
    description: 'Turn repeatable project routines into simple workflows you can run whenever you need them.',
    category: 'planning',
    id: 'automate',
    placement: 'view',
    settingsKey: 'automateViewTabHidden',
    title: 'Automate',
  },
  /**
   * CDXC:Bots 2026-09-26 DECISION:
   * User: Bots is an Official extension in Planning and automation, off by default, and offered only on a computer where the Hermes CLI is installed.
   */
  {
    appWide: true,
    description:
      'Swap the sidebar to your Hermes agents: one row each, with a new session one click away. Needs the Hermes CLI.',
    category: 'planning',
    id: 'bots',
    placement: 'sidebar',
    requiresAgentCli: 'hermes-agent',
    settingsKey: 'botsHidden',
    title: 'Bots',
  },
  /**
   * CDXC:Bots 2026-09-26 DECISION:
   * User: Bot automations is a second Official extension beside Bots, off by default, offered only where the Hermes CLI is installed, and disabled while Bots is off.
   * SEE-ALSO: packages/gx-core/src/sidebar_view/assemble.rs (the Automations row), apps/desktop/src/app/helpers/titlebar/mode_switcher.rs (`titlebar_mode_required_hidden_settings_key`) and server/src/bot_feed.rs (`read_bot_feed`) apply the same "needs Bots" rule.
   */
  {
    appWide: true,
    description: 'A feed of every Hermes cron run, one channel per job, opened from the Bots sidebar. Needs Bots.',
    category: 'planning',
    id: 'botAutomations',
    placement: 'view',
    requiresAgentCli: 'hermes-agent',
    requiresExtension: 'bots',
    settingsKey: 'botAutomationsHidden',
    title: 'Bot automations',
  },
  /**
   * CDXC:OpenDots 2026-10-01 DECISION:
   * User: OpenDots (Open Dot's personal agents) is a new extension on Settings > Extensions, built into Ghostex for now, that shows as a new Space in the sidebar when enabled. It is off by default like Bots.
   */
  {
    appWide: true,
    description:
      'Personal agents that work on their own: each dot has its own browser, memory, rules and routines, asks before anything risky, and shows as a Dots Space in the sidebar.',
    category: 'planning',
    id: 'openDots',
    placement: 'sidebar',
    settingsKey: 'openDotsHidden',
    title: 'OpenDots',
  },
  {
    description: 'Browse your project’s notes, plans, and reference files together in one focused reading space.',
    category: 'code-and-files',
    id: 'docs',
    placement: 'view',
    settingsKey: 'docsViewTabHidden',
    title: 'Files',
  },
  {
    description:
      'A command terminal beside your sessions, with its own tabs and splits, that works like the Commands pane but lives in the view panel.',
    category: 'code-and-files',
    id: 'terminal',
    placement: 'view',
    settingsKey: 'terminalViewTabHidden',
    title: 'Terminal',
  },
  {
    appWide: true,
    description: 'A panel of short tips for getting more out of Ghostex, opened from the ⋯ menu.',
    category: 'menus-and-panels',
    id: 'tips',
    placement: 'titlebar-button',
    settingsKey: 'tipsAndTricksTitlebarButtonHidden',
    title: 'Tips & Tricks',
  },
  {
    description: "A bell in the sidebar's top row that lists what your agents finished or need from you.",
    category: 'header-buttons',
    id: 'notifications',
    placement: 'titlebar-button',
    settingsKey: 'notificationsTitlebarButtonHidden',
    title: 'Notifications',
  },
  {
    appWide: true,
    description:
      'A ⋯ menu entry with sample questions that start a Ghostex Help chat: an agent explains the app or changes settings for you.',
    category: 'menus-and-panels',
    id: 'help',
    placement: 'titlebar-button',
    settingsKey: 'helpTitlebarButtonHidden',
    title: 'Ghostex Help',
  },
  {
    description:
      'A ⋯ menu panel listing development servers running on this computer. A new Browser tab shows the same list.',
    category: 'menus-and-panels',
    id: 'devServers',
    placement: 'titlebar-button',
    settingsKey: 'devServersTitlebarButtonHidden',
    title: 'Dev servers',
  },
  {
    appWide: true,
    description:
      'A ⋯ menu panel listing what Ghostex is running right now, with the CPU and memory each part is using.',
    category: 'menus-and-panels',
    id: 'resources',
    placement: 'titlebar-button',
    settingsKey: 'resourcesTitlebarButtonHidden',
    title: 'Resources',
  },
  {
    description: 'A work area header button for commit, branch, and worktree helpers on the active project.',
    category: 'header-buttons',
    id: 'gitActions',
    placement: 'titlebar-button',
    settingsKey: 'gitActionsTitlebarButtonHidden',
    title: 'Git actions',
  },
  {
    description: 'A work area header button that runs your saved terminal and browser actions in one click.',
    category: 'header-buttons',
    id: 'quickActions',
    placement: 'titlebar-button',
    settingsKey: 'quickActionsTitlebarButtonHidden',
    title: 'Quick Actions',
  },
  {
    description: 'A work area header button that opens the active project in another app.',
    category: 'header-buttons',
    id: 'openIn',
    placement: 'titlebar-button',
    settingsKey: 'openInTitlebarButtonHidden',
    title: 'Open In',
  },
  {
    description: 'An entry in the work area header’s ⋯ menu that opens this Extensions page.',
    category: 'menus-and-panels',
    id: 'extensionsButton',
    placement: 'titlebar-button',
    settingsKey: 'extensionsTitlebarButtonHidden',
    title: 'Extensions',
  },
];

/** An official entry is on when its inverted "hidden" settings key is not true. */
export function isOfficialExtensionEnabled(
  settings: ghostexSettings,
  extension: Pick<GhostexOfficialExtension, 'settingsKey'>
): boolean {
  return settings[extension.settingsKey] !== true;
}

/** The entry that has to be enabled first while it is off, or nothing when this entry can be switched freely. */
export function officialExtensionBlockedBy(
  settings: ghostexSettings,
  extension: Pick<GhostexOfficialExtension, 'requiresExtension'>
): GhostexOfficialExtension | undefined {
  const required = GHOSTEX_OFFICIAL_EXTENSIONS.find(({ id }) => id === extension.requiresExtension);
  return required && !isOfficialExtensionEnabled(settings, required) ? required : undefined;
}
