// One entry per part of the Coucou-parity work, so the bridge needn't know them one by one.
import * as island from './island/model';
import * as agents from './agents/model';
import * as flow from './flow/model';
import * as services from './services/model';
import * as mail from './mail/model';
import * as media from './media/model';
import * as utilities from './utilities/model';

export const streamDefaults = { island: island.defaults, agents: agents.defaults, flow: flow.defaults, services: services.defaults, mail: mail.defaults, media: media.defaults, utilities: utilities.defaults };

export const streamPreviews: Record<string, (op: never) => unknown> = {
  island_call: island.preview,
  agents_call: agents.preview,
  flow_call: flow.preview,
  services_call: services.preview,
  mail_call: mail.preview,
  media_call: media.preview,
  utilities_call: utilities.preview,
};
