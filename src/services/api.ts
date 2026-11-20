import { command } from '../bridge';
import type { ServiceId, ServicesOp, ServiceStatus } from './model';

export const callServices = <T>(op: ServicesOp) => command<T>('services_call', { op });

/** Every service's state. Asking counts as watching, so the services are polled while Settings
    shows them; ask only while it is on screen. */
export const serviceStatuses = () => callServices<ServiceStatus[]>({ op: 'status' });

/** Settings stopped showing the services: stop polling for it. */
export const unwatchServices = () => callServices<null>({ op: 'unwatch' });

/** Saves a key in the Keychain for its host, or removes it when empty. A self-hosted service's
    host is saved in the same step, and a new host removes the old key. */
export const saveServiceKey = (service: ServiceId, key: string, host?: string) => callServices<ServiceStatus[]>(host === undefined ? { op: 'saveKey', service, key } : { op: 'saveKey', service, key, host });

/** Shows a service on the island again. */
export const showService = (service: ServiceId) => callServices<ServiceStatus[]>({ op: 'show', service });
