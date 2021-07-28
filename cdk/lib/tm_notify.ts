import * as core from "@aws-cdk/core";
import * as s3 from '@aws-cdk/aws-s3'
import * as lambda from '@aws-cdk/aws-lambda'
import * as iam from '@aws-cdk/aws-iam'

export class TmNotify extends core.Construct {
    constructor(scope: core.Construct, id: string) {
        super(scope, id);

        const bucket = new s3.Bucket(this, "TmNotifyAssets");

        // This user will be used to publish artifacts from Github actions.
        const ghUser = new iam.User(this, "TmGithubUser");
        bucket.grantWrite(ghUser);

        // const handler = new lambda.Function(this, "TmNotify", {
        //     runtime: lambda.Runtime.PROVIDED_AL2,
        //     handler: "custom.runtime",
        //     code: lambda.Code.fromBucket(bucket, "release/latest.zip")
        // });
    }
}
