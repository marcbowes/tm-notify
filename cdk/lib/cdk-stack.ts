import * as cdk from '@aws-cdk/core';
import * as tm_notify from '../lib/tm_notify';

export class CdkStack extends cdk.Stack {
  constructor(scope: cdk.Construct, id: string, props?: cdk.StackProps) {
    super(scope, id, props);

    new tm_notify.TmNotify(this, "TmNotify");
  }
}
