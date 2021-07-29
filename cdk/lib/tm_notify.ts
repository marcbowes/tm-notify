import * as core from "@aws-cdk/core";
import * as s3 from '@aws-cdk/aws-s3'
import * as lambda from '@aws-cdk/aws-lambda'
import * as iam from '@aws-cdk/aws-iam'
import * as events from '@aws-cdk/aws-events';
import * as targets from '@aws-cdk/aws-events-targets';

export class TmNotify extends core.Construct {
    constructor(scope: core.Construct, id: string) {
        super(scope, id);

        const lambdaFunction = new lambda.Function(this, "TmNotifyHandler", {
            runtime: lambda.Runtime.PROVIDED_AL2,
            handler: "custom.runtime",
            code: lambda.Code.fromAsset("dummy-lambda-code.zip")
        });

        const rule = new events.Rule(this, 'Rule', {
            schedule: events.Schedule.expression('rate(5 minutes)')
        });

        rule.addTarget(new targets.LambdaFunction(lambdaFunction));

        // This user will be used to update the Lambda function via Github
        // actions. The intial "code from asset" is simply because we *don't*
        // have an initial version that we can upload. Github actions builds on
        // linux, links with musl, etc. We can't do that locally.
        const ghUser = new iam.User(this, "TmNotifyGithubUser");

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

        const appBucket = new s3.Bucket(this, "TmNotifyVar");
        appBucket.grantReadWrite(lambdaFunction);
    }
}
