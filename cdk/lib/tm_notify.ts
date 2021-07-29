import * as core from "@aws-cdk/core";
import * as s3 from '@aws-cdk/aws-s3'
import * as lambda from '@aws-cdk/aws-lambda'
import * as iam from '@aws-cdk/aws-iam'

export class TmNotify extends core.Construct {
    constructor(scope: core.Construct, id: string) {
        super(scope, id);

        const bucket = new s3.Bucket(this, "TmNotifyAssets");
        const artifactKey = "release/latest.zip";

        // This user will be used to update the Lambda function via Github actions.
        const ghUser = new iam.User(this, "TmNotifyGithubUser");
        bucket.grantWrite(ghUser);

        const lambdaFunction = new lambda.Function(this, "TmNotifyHandler", {
            runtime: lambda.Runtime.PROVIDED_AL2,
            handler: "custom.runtime",
            code: lambda.Code.fromBucket(bucket, artifactKey)
        });

        new iam.Policy(this, "TmNotifyGithubUserUpdateFunctionPolicy", {
            statements: [
                new iam.PolicyStatement({
                    effect: iam.Effect.ALLOW,
                    resources: [lambdaFunction.functionArn],
                    actions: ["lambda:UpdateFunctionCode"]
                })
            ],
            users: [ghUser]
        });
    }
}
